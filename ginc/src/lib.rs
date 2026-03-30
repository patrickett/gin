mod args;
pub mod ast;
pub mod codegen;
pub mod compilation;
pub mod database;
pub mod diagnostic;
pub mod intern;
pub mod lexer;
pub mod parse;
pub mod source;
pub mod typeck;

use crate::typeck::TyEnv;
pub use args::*;
pub use ast::{DefMap, FileAst, Symbol, SymbolKind, SymbolTable, TagMap};
pub use compilation::completions::{
    CompletionCandidate, CompletionKind, SignatureInfo, completions_for_ast, fn_call_at,
    format_params, signature_for_fn,
};
pub use compilation::hover::{find_definition_span, find_references};
pub use database::{
    File,
    input_database::{Db, InputDatabase},
};
pub use diagnostic::{Category, Symptom, SymptomSource};
pub use lexer::is_comment_at;
pub use source::{
    get_char_at_position, get_number_at_position, get_word_at_position, is_identifier_char,
    is_in_comment, position_to_byte_offset, word_at_byte_offset,
};

use crate::compilation::{compile::compile_entry, native};
use crossbeam_channel::unbounded;
use std::path::{Path, PathBuf};

pub const GIN_FILE_EXT: &str = "gin";

pub mod prelude {
    pub use crate::ast::*;
    pub use crate::codegen::{CodegenContext, Lower, RuntimeSymbolTable};
    pub use crate::intern::IStr;
    pub use crate::lexer::{MAX_INDENT_DEPTH, Token};
    pub use crate::parse::ParserError;
    pub use chumsky::{input::ValueInput, prelude::*};
}

/// Analagous to the `ginc` command
pub struct GinCompiler;

impl GinCompiler {
    /// Compile a Gin project.
    ///
    /// If the input path is a single `.gin` file, it is treated as a binary
    /// entry point: imports are resolved transitively and the result is linked
    /// into an executable (or object file / MLIR text, depending on `--emit`).
    ///
    /// If the input path is a directory, all `.gin` files within it are treated
    /// as a single library unit: they are parsed, type-checked, and
    /// flow-analyzed together with a shared type environment, then compiled
    /// into one object file.
    pub fn compile(args: &'_ mut Args) {
        let path = args.input.to_owned();
        let is_library = path.is_dir();

        // ── Phase 1: Collect file paths ───────────────────────────────
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

        let (db_files, filenames) = match load_files_into_db(&db, &file_paths) {
            Some(result) => result,
            None => return,
        };

        // ── Phase 3: Dispatch to mode-specific pipeline ───────────────
        if is_library {
            compile_library_pipeline(&db, &db_files, &filenames, args, &path);
        } else {
            compile_binary_pipeline(&db, db_files, &filenames, args, &path);
        }
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────────

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

/// Collect all .gin file paths in a directory recursively, skipping `target/`.
fn collect_gin_files_recursive(dir: &Path) -> Vec<PathBuf> {
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

// ── Library pipeline ────────────────────────────────────────────────────────

/// Compile a library directory (all .gin files) to an object file.
///
/// Every file is parsed, type-checked, and flow-analyzed through the Salsa
/// pipeline with a **shared type environment** built from the merged AST of
/// all files. This means types defined in any file are visible to all other
/// files — matching the semantics of a single-compilation-unit library.
fn compile_library_pipeline(
    db: &InputDatabase,
    db_files: &[File],
    filenames: &[String],
    args: &Args,
    path: &Path,
) {
    let all_files: Vec<File> = db_files.to_vec();

    // Run full analysis (parse + type check + flow analysis) per file.
    // Each call uses a merged TyEnv so cross-file types are visible.
    for &file in db_files {
        crate::compilation::compile::analyze_file_in_library(db, file, all_files.clone());
    }

    // Collect and print diagnostics per-file with correct source context.
    let has_flaws = print_library_diagnostics(db, db_files, filenames, &all_files);
    if has_flaws {
        return;
    }

    // Build merged AST for codegen (parse results are cached by Salsa).
    use crate::parse::parse::parse as salsa_parse;
    let mut merged_ast = FileAst::default();
    for &file in db_files {
        merged_ast.merge_from(salsa_parse(db, file));
    }

    // Determine output path.
    let obj_path = args.output.clone().unwrap_or_else(|| {
        let pkg_name = path.file_name().unwrap_or_default().to_string_lossy();
        path.join("target").join(format!("{}.o", pkg_name))
    });

    // Ensure target directory exists.
    if let Some(parent) = obj_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Compile merged AST to object file.
    let dir_name = path.to_string_lossy().into_owned();
    let ty_env = TyEnv::from_file_ast(&merged_ast);
    match native::compile_to_object(&merged_ast, &obj_path, args.profile, "", &dir_name, &ty_env) {
        Ok(()) => println!("Compiled library to {}", obj_path.display()),
        Err(e) => eprintln!("Codegen error: {e:?}"),
    }
}

/// Print diagnostics for each library file with the correct source context.
///
/// Because `analyze_file_in_library` is a per-file tracked function, each
/// invocation's accumulated symptoms naturally belong to that file.
fn print_library_diagnostics(
    db: &InputDatabase,
    db_files: &[File],
    filenames: &[String],
    all_files: &[File],
) -> bool {
    let mut has_flaws = false;
    let all_files_vec: Vec<File> = all_files.to_vec();

    for (i, &file) in db_files.iter().enumerate() {
        let source = file.contents(db);
        let filename = &filenames[i];

        let diagnostics = crate::compilation::compile::analyze_file_in_library::accumulated::<
            Symptom,
        >(db, file, all_files_vec.clone());

        for diagnostic in &diagnostics {
            diagnostic.print(source, filename);
            if matches!(diagnostic.category, Category::Flaw) {
                has_flaws = true;
            }
        }
    }

    has_flaws
}

// ── Binary pipeline ─────────────────────────────────────────────────────────

/// Compile a single entry-point file and all its transitive imports.
///
/// Uses the Salsa-tracked `compile_entry` query which orchestrates type
/// checking, flow analysis, and code generation for the full dependency
/// graph.
fn compile_binary_pipeline(
    db: &InputDatabase,
    db_files: Vec<File>,
    filenames: &[String],
    args: &mut Args,
    path: &Path,
) {
    let entry = db_files[0];

    // Compile entry point and all its imports through the Salsa pipeline.
    let compiled = compile_entry(db, entry);

    // Collect diagnostics from the full dependency graph.
    let source = entry.contents(db).to_string();
    let filename = filenames[0].clone();
    let diagnostics = compile_entry::accumulated::<Symptom>(db, entry);
    let has_flaws = diagnostics
        .iter()
        .any(|d| matches!(d.category, diagnostic::Category::Flaw));

    for diagnostic in &diagnostics {
        diagnostic.print(&source, &filename);
    }

    if has_flaws {
        return;
    }

    // Retrieve MLIR output from the compiled module.
    let bytecode = compiled.bytecode(db);
    if bytecode.is_empty() {
        eprintln!("Compilation failed or produced no output");
        return;
    }
    let mlir_text = String::from_utf8_lossy(bytecode);

    // Emit according to the requested output kind.
    match args.emit {
        Emit::Mlir => {
            println!("\n```mlir\n{mlir_text}```\n");
        }
        Emit::Obj | Emit::Exe => {
            let obj_path = if matches!(args.emit, Emit::Exe) {
                let exe_path = args
                    .output
                    .clone()
                    .unwrap_or_else(|| path.with_extension(""));
                exe_path.with_extension("o")
            } else {
                args.output
                    .clone()
                    .unwrap_or_else(|| path.with_extension("o"))
            };

            if let Err(e) = native::native_from_mlir(&mlir_text, &obj_path, args.profile) {
                eprintln!("Codegen error: {e:?}");
                return;
            }

            if matches!(args.emit, Emit::Exe) {
                let exe_path = args
                    .output
                    .clone()
                    .unwrap_or_else(|| path.with_extension(""));
                if let Err(e) =
                    native::link_executable(&obj_path, &exe_path, args.target.as_deref())
                {
                    eprintln!("Link error: {e:?}");
                }
                let _ = std::fs::remove_file(&obj_path);
            }
        }
    }
}
