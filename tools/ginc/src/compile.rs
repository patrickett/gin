//! Compilation orchestration.

use crate::cli::Args;
use ast::{
    FileId, TypedFileAst,
    typed::{TransformCtx, transform_file_with_ctx},
};
use codegen::emit;
use diagnostic::{Category, Diagnostic};
use flask::FlaskConfig;
use lexer::debug_tokens;
use parser::parse_source_full;
use resolve::ParsedFile;
use std::path::{Path, PathBuf};

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

            resolve::resolve_imports(files, &args.dependencies)
        } else {
            files
        };

        if print_diagnostics(&files) {
            return;
        }

        // Transform files into TypedFileAsts with cross-file resolution.
        // No merge step — each file stays independent. Codegen consumes
        // the first file's typed AST (entry file for binaries, or primary for libraries).
        let typed_asts: Vec<TypedFileAst> = {
            let mut results: Vec<TypedFileAst> = Vec::with_capacity(files.len());
            for (i, f) in files.iter().enumerate() {
                let ctx = TransformCtx::from_typed_asts(&results);
                let typed = transform_file_with_ctx(f.output.ast.clone(), FileId(i as u32), &ctx);
                results.push(typed);
            }
            results
        };

        // Print type-check flaws from the typed AST (uses the diagnostic crate for proper
        // messages, help text, and ariadne rendering).
        //
        // Type flaws are printed but do NOT gate compilation — type checking is
        // best-effort diagnostics; codegen may still succeed for code the checker
        // doesn't fully understand yet (e.g. template unions).
        print_type_diagnostics(&files, &typed_asts);

        match args.emit {
            crate::cli::Emit::Mlir => emit_mlir_typed(&files, &typed_asts),
            crate::cli::Emit::Obj | crate::cli::Emit::Exe => {
                emit_native_typed(&files, &typed_asts, args, &path, is_library)
            }
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

// Delegates to resolve::collect_gin_files (single source of truth).
fn collect_gin_files(root: &Path) -> Vec<PathBuf> {
    resolve::collect_gin_files(root)
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

/// Print type-check flaws from the typed AST for every file.
///
/// Flaws are already [`diagnostic::TypeSymptom`] values — they are converted to
/// [`Diagnostic`] via the [`DiagnosticLike`](diagnostic::DiagnosticLike) trait and
/// rendered with the same ariadne-based printer used for parse diagnostics.
fn print_type_diagnostics(files: &[ParsedFile], typed_asts: &[TypedFileAst]) {
    use diagnostic::DiagnosticLike;

    for (file, typed) in files.iter().zip(typed_asts) {
        let filename = path_for_diagnostic_report(&file.path);
        let span_table = file.output.ast.span_table();
        for (expr_id, flaw) in typed.all_flaws() {
            let span_id = typed.exprs.span[expr_id.as_usize()];
            let diag = flaw.clone().into_diagnostic(span_id);
            diag.print(span_table, &file.source, &filename);
        }
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

/// Print MLIR text to stdout using the typed AST (no merge step).
fn emit_mlir_typed(files: &[ParsedFile], typed_asts: &[TypedFileAst]) {
    let Some(typed) = typed_asts.first() else {
        return;
    };
    let (source, label) = match files.first() {
        Some(f) => (f.source.as_str(), path_for_diagnostic_report(&f.path)),
        None => ("", "<stdin>".to_string()),
    };
    let (result, symptoms) = emit::build_module_text_from_typed(typed, source, &label);
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

/// Compile to object file / executable using the typed AST (no merge step).
fn emit_native_typed(
    files: &[ParsedFile],
    typed_asts: &[TypedFileAst],
    args: &Args,
    path: &Path,
    is_library: bool,
) {
    let (Some(typed), Some(file)) = (typed_asts.first(), files.first()) else {
        return;
    };
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

    let source = file.source.as_str();
    let label = path_for_diagnostic_report(&file.path);
    let profile = args.profile.into();
    let (ok, symptoms) =
        emit::compile_to_object_from_typed(typed, &obj_path, profile, source, &label);
    if !ok {
        eprintln!("Codegen failed: {:?}", symptoms);
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
            for s in &link_symptoms {
                eprintln!("Link error: {}", s.message);
            }
        }
        let _ = std::fs::remove_file(&obj_path);
    } else {
        println!("Compiled to {}", obj_path.display());
    }
}
