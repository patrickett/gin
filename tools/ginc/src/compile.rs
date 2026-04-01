//! Compilation orchestration — the main compiler pipeline.

use crate::cli::Args;
use ast::FileAst;
use codegen::emit::native;
use crossbeam_channel::unbounded;
use database::{
    File,
    input_database::{Db, InputDatabase},
};
use diagnostic::{Category, Symptom};
use lexer::debug_tokens;
use std::path::{Path, PathBuf};
use typeck::TyEnv;
use typeck::{analyze_file, analyze_package};

/// Analogous to the `ginc` command
pub struct GinCompiler;

impl GinCompiler {
    /// Compile a Gin project through a unified pipeline.
    ///
    /// **Binary mode** (input is a `.gin` file):
    /// Imports are resolved transitively and the result is linked into an
    /// executable (or object file / MLIR text, depending on `--emit`).
    ///
    /// **Library mode** (input is a directory):
    /// All `.gin` files are treated as a single compilation unit with a
    /// shared type environment, compiled into one object file.
    pub fn compile(args: &'_ mut Args) {
        let path = args.input.to_owned();
        let is_library = path.is_dir();

        // ── Phase 1: Collect source file paths ────────────────────────
        let file_paths = if is_library {
            collect_gin_files_recursive(&path)
        } else {
            vec![path.clone()]
        };

        if file_paths.is_empty() {
            eprintln!("No .gin files found in {}", path.display());
            return;
        }

        // ── Phase 2: Create database and load files ───────────────────
        let (tx, _rx) = unbounded();
        let db = InputDatabase::new(tx);

        let (db_files, _filenames) = match load_files_into_db(&db, &file_paths) {
            Some(result) => result,
            None => return,
        };

        if matches!(args.emit, crate::cli::Emit::Tokens) {
            for file in &db_files {
                let source = file.contents(&db);
                print!("{}", debug_tokens(source));
            }
            return;
        }

        // ── Phase 3: Resolve imports (binary mode only) ───────────────
        let all_files = if is_library {
            db_files.clone()
        } else {
            resolve_all_imports(&db, db_files[0])
        };

        // ── Phase 4: Analyze (parse + type check + flow analysis) ─────
        let asts = analyze_package(&db, all_files.clone());

        // ── Phase 5: Print diagnostics ────────────────────────────────
        if print_diagnostics(&db, &all_files) {
            return;
        }

        // ── Phase 6: Build merged AST for codegen ─────────────────────
        let mut merged_ast = FileAst::default();
        for ast in &asts {
            merged_ast.merge_from(ast.clone());
        }
        let ty_env = TyEnv::from_file_ast(&merged_ast);

        // ── Phase 7: Emit output ──────────────────────────────────────
        match args.emit {
            crate::cli::Emit::Mlir => emit_mlir(&merged_ast, &ty_env),
            crate::cli::Emit::Obj | crate::cli::Emit::Exe => {
                emit_native(&merged_ast, &ty_env, args, &path, is_library)
            }
            crate::cli::Emit::Tokens => unreachable!(),
        }
    }
}

// ── File Collection ─────────────────────────────────────────────────────────

/// Collect all .gin file paths in a directory recursively, skipping `target/`.
pub fn collect_gin_files_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            files.extend(collect_gin_files_recursive(&path));
        } else if path.extension().is_some_and(|ext| ext == "gin") {
            files.push(path);
        }
    }

    files
}

// ── Database Loading ────────────────────────────────────────────────────────

/// Load file paths into the Salsa database, returning the handles and display names.
fn load_files_into_db(db: &InputDatabase, paths: &[PathBuf]) -> Option<(Vec<File>, Vec<String>)> {
    let mut files = Vec::with_capacity(paths.len());
    let mut filenames = Vec::with_capacity(paths.len());

    for path in paths {
        let filename = path.to_string_lossy().into_owned();
        match db.input(path.clone()) {
            Ok(file) => {
                files.push(file);
                filenames.push(filename);
            }
            Err(err) => {
                eprintln!("Error loading {}: {}", path.display(), err);
                return None;
            }
        }
    }

    Some((files, filenames))
}

// ── Import Resolution ───────────────────────────────────────────────────────

/// Resolve transitive imports starting from an entry file.
fn resolve_all_imports(db: &InputDatabase, entry: File) -> Vec<File> {
    let imported = resolve_imports(db, entry);

    let mut all_files = vec![entry];
    let mut seen = vec![entry.path(db)];

    for file in imported {
        let path = file.path(db);
        if !seen.contains(&path) {
            seen.push(path);
            all_files.push(file);
        }
    }

    all_files
}

/// Resolve imports from a single file.
fn resolve_imports(db: &InputDatabase, entry: File) -> Vec<File> {
    use ast::resolve_imports;
    resolve_imports(db, entry)
}

// ── Diagnostics ─────────────────────────────────────────────────────────────

/// Print diagnostics for all files with correct per-file source context.
///
/// Returns `true` if any fatal flaws were found.
fn print_diagnostics(db: &InputDatabase, all_files: &[File]) -> bool {
    let mut has_flaws = false;
    let all_files_vec: Vec<File> = all_files.to_vec();

    for &file in all_files {
        let source = file.contents(db);
        let filename = file.path(db).to_string_lossy().into_owned();

        // Parse symptoms (accumulated per-file by the parse query)
        for diagnostic in ast::parse_file::accumulated::<Symptom>(db, file) {
            diagnostic.print(source, &filename);
            if matches!(diagnostic.category, Category::Flaw) {
                has_flaws = true;
            }
        }

        // Analysis symptoms (type check + flow analysis, per-file)
        for diagnostic in analyze_file::accumulated::<Symptom>(db, file, all_files_vec.clone()) {
            diagnostic.print(source, &filename);
            if matches!(diagnostic.category, Category::Flaw) {
                has_flaws = true;
            }
        }
    }

    has_flaws
}

// ── Codegen & Output ────────────────────────────────────────────────────────

/// Print MLIR text to stdout.
fn emit_mlir(merged_ast: &FileAst, ty_env: &TyEnv) {
    match native::build_module_text(merged_ast, "", "<stdin>", ty_env) {
        Ok(mlir_text) => println!("\n```mlir\n{mlir_text}```\n"),
        Err(e) => eprintln!("Codegen error: {e:?}"),
    }
}

/// Compile to object file / executable.
fn emit_native(merged_ast: &FileAst, ty_env: &TyEnv, args: &Args, path: &Path, is_library: bool) {
    // Determine object file path
    let obj_path = if is_library {
        args.output.clone().unwrap_or_else(|| {
            let pkg_name = path.file_name().unwrap_or_default().to_string_lossy();
            path.join("target").join(format!("{}.o", pkg_name))
        })
    } else if matches!(args.emit, crate::cli::Emit::Exe) {
        args.output
            .clone()
            .unwrap_or_else(|| path.with_extension(""))
            .with_extension("o")
    } else {
        args.output
            .clone()
            .unwrap_or_else(|| path.with_extension("o"))
    };

    // Ensure target directory exists
    if let Some(parent) = obj_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Compile merged AST to object file
    let filename = path.to_string_lossy();
    let profile = args.profile.into();
    match native::compile_to_object(merged_ast, &obj_path, profile, "", &filename, ty_env) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Codegen error: {e:?}");
            return;
        }
    }

    // Link into executable if requested
    if matches!(args.emit, crate::cli::Emit::Exe) {
        let exe_path = args
            .output
            .clone()
            .unwrap_or_else(|| path.with_extension(""));
        if let Err(e) = native::link_executable(&obj_path, &exe_path, args.target.as_deref()) {
            eprintln!("Link error: {e:?}");
        }
        let _ = std::fs::remove_file(&obj_path);
    } else {
        println!("Compiled to {}", obj_path.display());
    }
}
