//! Compilation orchestration.

use crate::cli::Args;
use ast::FileAst;
use codegen::emit;
use diagnostic::{Category, Diagnostic};
use flask::{FlaskConfig, PACKAGE_CONFIG_NAME};
use internment::Intern;
use lexer::debug_tokens;
use parser::parse_source_full;
use resolve::ParsedFile;
use std::path::{Path, PathBuf};
use typeck::TyEnv;

struct TypecheckResult {
    ty_env: TyEnv,
    symptoms: Vec<Vec<Diagnostic>>,
}

/// Analogous to the `ginc` command
pub struct GinCompiler;

impl GinCompiler {
    /// Compile a Gin project through a staged pipeline.
    ///
    /// **Binary mode** (input is a `.gin` file):
    /// Imports are resolved and the result is linked into an
    /// executable (or object file / MLIR text, depending on `--emit`).
    ///
    /// **Library mode** (input is a directory):
    /// All `.gin` files are treated as a single compilation unit with a
    /// shared type environment, compiled into one object file.
    pub fn compile(args: &'_ mut Args) {
        let path = args.input.to_owned();

        let file_paths = collect_gin_files(&path);
        if file_paths.is_empty() {
            eprintln!("No .gin files found in {}", path.display());
            return;
        }

        let is_library = path.is_dir();
        let sources = read_sources(&file_paths);

        let files = parse(&sources);

        // Early exit for token dump
        if matches!(args.emit, crate::cli::Emit::Tokens) {
            for file in &files {
                print!("{}", debug_tokens(&file.source));
            }
            return;
        }

        if print_diagnostics(&files) {
            return;
        }

        let files = if !is_library {
            let entry_dir = files[0]
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            if args.dependencies.is_empty()
                && let Some(config) = FlaskConfig::from_directory(&entry_dir)
            {
                args.dependencies = resolve::resolve_flask_path_dependencies(&config, &entry_dir);
            }
            resolve::resolve_imports(files, Some(&args.dependencies))
        } else {
            files
        };

        if print_diagnostics(&files) {
            return;
        }

        let checked = typecheck(&files);

        if print_typecheck_diagnostics(&files, &checked) {
            return;
        }

        let merged_ast = match resolve::merge_asts_checked(&files) {
            Ok(m) => m,
            Err(symptoms) => {
                print_standalone_import_diagnostics(&files, &symptoms);
                return;
            }
        };

        if matches!(args.emit, crate::cli::Emit::Exe)
            && !is_library
            && !validate_main_binary(&merged_ast, &path)
        {
            return;
        }

        match args.emit {
            crate::cli::Emit::Mlir => emit_mlir(&files, &merged_ast, &checked.ty_env),
            crate::cli::Emit::Obj | crate::cli::Emit::Exe => emit_native(
                &files,
                &merged_ast,
                &checked.ty_env,
                args,
                &path,
                is_library,
            ),
            crate::cli::Emit::Tokens => unreachable!(),
        }
    }
}

fn parse(sources: &[(PathBuf, String)]) -> Vec<ParsedFile> {
    sources
        .iter()
        .map(|(path, source)| {
            let output = parse_source_full(source);
            ParsedFile {
                path: path.clone(),
                source: source.clone(),
                output,
            }
        })
        .collect()
}

fn typecheck(files: &[ParsedFile]) -> TypecheckResult {
    let asts: Vec<_> = files.iter().map(|f| f.output.ast.clone()).collect();
    let ty_env = TyEnv::from_multiple_file_asts(&asts);

    let symptoms = asts
        .iter()
        .map(|ast| typeck::analyze_file_with_ty_env(ast, &ty_env))
        .collect();

    TypecheckResult { ty_env, symptoms }
}

/// Collect `.gin` file paths under `root`, skipping `target/` directories.
///
/// If `root` is a folder module (contains `flask.jsonc`), only immediate `*.gin` files are used.
fn collect_gin_files(root: &Path) -> Vec<PathBuf> {
    if root.is_dir() {
        if root.join(PACKAGE_CONFIG_NAME).is_file() {
            flask::list_package_gin_files(root)
        } else {
            collect_gin_files_recursive(root)
        }
    } else {
        vec![root.to_path_buf()]
    }
}

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

/// Read file contents from disk, skipping files that can't be read.
/// Path shown in ariadne diagnostics: relative to the process current directory when possible.
fn path_for_diagnostic_report(path: &Path) -> String {
    let Ok(cwd) = std::env::current_dir() else {
        return path.display().to_string();
    };
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    match (abs.canonicalize(), cwd.canonicalize()) {
        (Ok(abs), Ok(base)) => abs
            .strip_prefix(&base)
            .map(|p| {
                let s = p.display().to_string();
                if s.is_empty() { ".".to_string() } else { s }
            })
            .unwrap_or_else(|_| abs.display().to_string()),
        _ => path.display().to_string(),
    }
}

fn read_sources(paths: &[PathBuf]) -> Vec<(PathBuf, String)> {
    let mut sources = Vec::with_capacity(paths.len());
    for fp in paths {
        match std::fs::read_to_string(fp) {
            Ok(s) => sources.push((fp.clone(), s)),
            Err(err) => eprintln!("Error reading {}: {}", fp.display(), err),
        }
    }
    sources
}

/// Print all diagnostics for a slice of parsed files.
///
/// Each file's symptoms are printed with its own span table and source text.
/// Returns `true` if any fatal diagnostics were found.
fn print_diagnostics(files: &[ParsedFile]) -> bool {
    let mut has_flaws = false;
    for file in files {
        let filename = path_for_diagnostic_report(&file.path);
        let span_table = file.output.ast.span_table();
        for diag in &file.output.symptoms {
            diag.print(span_table, &file.source, &filename);
            if matches!(diag.category, Category::Flaw) {
                has_flaws = true;
            }
        }
    }
    has_flaws
}

/// Print type-check diagnostics returned by the typecheck stage.
fn print_typecheck_diagnostics(files: &[ParsedFile], result: &TypecheckResult) -> bool {
    let mut has_flaws = false;
    for (i, symptoms) in result.symptoms.iter().enumerate() {
        let file = &files[i];
        let filename = path_for_diagnostic_report(&file.path);
        let span_table = file.output.ast.span_table();
        for diag in symptoms {
            diag.print(span_table, &file.source, &filename);
            if matches!(diag.category, Category::Flaw) {
                has_flaws = true;
            }
        }
    }
    has_flaws
}

fn validate_main_binary(merged: &FileAst, input: &Path) -> bool {
    let is_main_entry = input.file_name().is_some_and(|n| n == "main.gin");
    if !is_main_entry {
        return true;
    }
    let main_name = Intern::<String>::from_ref("main");
    if merged.defs.contains_key(&main_name) {
        return true;
    }
    eprintln!(
        "error: binary entry main.gin must define a top-level `main` binding (see {})",
        input.display()
    );
    false
}

fn print_standalone_import_diagnostics(files: &[ParsedFile], symptoms: &[Diagnostic]) {
    let Some(primary) = files.first() else {
        for s in symptoms {
            eprintln!(
                "{}: [{}] {}",
                s.category.as_str(),
                s.error_code(),
                s.message
            );
        }
        return;
    };
    let label = path_for_diagnostic_report(&primary.path);
    let span_table = primary.output.ast.span_table();
    let source = primary.source.as_str();
    for d in symptoms {
        d.print(span_table, source, &label);
    }
}

/// Print codegen / link diagnostics with the same ariadne layout as parse and type errors.
///
/// Uses the first parsed `.gin` file as source context (span table + text). This matches
/// lowering, which is keyed off the compilation entry file.
fn print_codegen_diagnostics(files: &[ParsedFile], symptoms: &[Diagnostic]) {
    if symptoms.is_empty() {
        return;
    }
    let Some(primary) = files.first() else {
        for s in symptoms {
            eprintln!(
                "{}: [{}] {}",
                s.category.as_str(),
                s.error_code(),
                s.message
            );
        }
        return;
    };
    let label = path_for_diagnostic_report(&primary.path);
    let span_table = primary.output.ast.span_table();
    let source = primary.source.as_str();
    for d in symptoms {
        d.print(span_table, source, &label);
    }
}

/// Print MLIR text to stdout.
fn emit_mlir(files: &[ParsedFile], merged_ast: &FileAst, ty_env: &TyEnv) {
    let (source, label) = match files.first() {
        Some(f) => (f.source.as_str(), path_for_diagnostic_report(&f.path)),
        None => ("", "<stdin>".to_string()),
    };
    let (result, symptoms) = emit::build_module_text(merged_ast, source, &label, ty_env);
    match result {
        Some(mlir_text) => {
            print_codegen_diagnostics(files, &symptoms);
            println!("\n```mlir\n{mlir_text}```\n");
        }
        None => {
            print_codegen_diagnostics(files, &symptoms);
        }
    }
}

/// Compile to object file / executable.
fn emit_native(
    files: &[ParsedFile],
    merged_ast: &FileAst,
    ty_env: &TyEnv,
    args: &Args,
    path: &Path,
    is_library: bool,
) {
    let obj_path = if is_library {
        // Folder packages reuse `args.output` only for `-o exe`/`link` destinations.
        // When emitting Exe we always stage `.o` under `<pkg>/target/` so `-o foo`
        // is never scribbled onto as raw object contents (which corrupts linkage).
        if matches!(args.emit, crate::cli::Emit::Exe) {
            let pkg_name = path.file_name().unwrap_or_default().to_string_lossy();
            path.join("target").join(format!("{pkg_name}.o"))
        } else {
            args.output.clone().unwrap_or_else(|| {
                let pkg_name = path.file_name().unwrap_or_default().to_string_lossy();
                path.join("target").join(format!("{}.o", pkg_name))
            })
        }
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

    if let Some(parent) = obj_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let (source, label) = match files.first() {
        Some(f) => (f.source.as_str(), path_for_diagnostic_report(&f.path)),
        None => ("", path.to_string_lossy().into_owned()),
    };
    let profile = args.profile.into();
    let (ok, symptoms) =
        emit::compile_to_object(merged_ast, &obj_path, profile, source, &label, ty_env);
    if !ok {
        print_codegen_diagnostics(files, &symptoms);
        return;
    }

    if matches!(args.emit, crate::cli::Emit::Exe) {
        let exe_path = args
            .output
            .clone()
            .unwrap_or_else(|| path.with_extension(""));
        let (linked, link_symptoms) =
            emit::link_executable(&obj_path, &exe_path, args.target.as_deref());
        if !linked {
            print_codegen_diagnostics(files, &link_symptoms);
        }
        let _ = std::fs::remove_file(&obj_path);
    } else {
        print_codegen_diagnostics(files, &symptoms);
        println!("Compiled to {}", obj_path.display());
    }
}
