//! Compilation orchestration — thin wrapper around the pipeline crate.

use crate::cli::Args;
use ast::FileAst;
use codegen::emit::native;
use flask::FlaskConfig;
use lexer::debug_tokens;
use pipeline::TypecheckResult;
use std::path::Path;
use typeck::TyEnv;

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

        let collection = pipeline::collect(&path);
        if collection.file_paths.is_empty() {
            eprintln!("No .gin files found in {}", path.display());
            return;
        }

        let is_library = collection.is_library;

        let parsed = pipeline::parse(collection);

        // Early exit for token dump
        if matches!(args.emit, crate::cli::Emit::Tokens) {
            for file in &parsed.files {
                print!("{}", debug_tokens(&file.source));
            }
            return;
        }

        if pipeline::print_diagnostics(&parsed.files) {
            return;
        }

        let deps = if !is_library {
            let entry_dir = parsed.files[0]
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            if args.dependencies.is_empty() {
                if let Some(config) = FlaskConfig::from_directory(&entry_dir) {
                    args.dependencies =
                        pipeline::resolve::resolve_flask_path_dependencies(&config, &entry_dir);
                }
            }
            Some(&args.dependencies)
        } else {
            None
        };

        let resolved = pipeline::resolve_imports(parsed, deps);

        if pipeline::print_diagnostics(&resolved.files) {
            return;
        }

        let checked = pipeline::typecheck(&resolved);

        if pipeline::print_diagnostics(&checked.files) {
            return;
        }

        let merged_ast = merge_asts(&checked);
        match args.emit {
            crate::cli::Emit::Mlir => emit_mlir(&merged_ast, &checked.ty_env),
            crate::cli::Emit::Obj | crate::cli::Emit::Exe => {
                emit_native(&merged_ast, &checked.ty_env, args, &path, is_library)
            }
            crate::cli::Emit::Tokens => unreachable!(),
        }
    }
}

fn merge_asts(checked: &TypecheckResult) -> FileAst {
    let mut merged = FileAst::default();
    for file in &checked.files {
        merged.merge_from(file.output.ast.clone());
    }
    merged
}

/// Print MLIR text to stdout.
fn emit_mlir(merged_ast: &FileAst, ty_env: &TyEnv) {
    let (result, symptoms) = native::build_module_text(merged_ast, "", "<stdin>", ty_env);
    match result {
        Some(mlir_text) => {
            for s in &symptoms {
                eprintln!("Codegen warning: [{}] {}", s.error_code(), s.message);
            }
            println!("\n```mlir\n{mlir_text}```\n");
        }
        None => {
            for s in &symptoms {
                eprintln!("Codegen error: [{}] {}", s.error_code(), s.message);
            }
        }
    }
}

/// Compile to object file / executable.
fn emit_native(merged_ast: &FileAst, ty_env: &TyEnv, args: &Args, path: &Path, is_library: bool) {
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

    if let Some(parent) = obj_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let filename = path.to_string_lossy();
    let profile = args.profile.into();
    let (ok, symptoms) =
        native::compile_to_object(merged_ast, &obj_path, profile, "", &filename, ty_env);
    if !ok {
        for s in &symptoms {
            eprintln!("Codegen error: [{}] {}", s.error_code(), s.message);
        }
        return;
    }

    if matches!(args.emit, crate::cli::Emit::Exe) {
        let exe_path = args
            .output
            .clone()
            .unwrap_or_else(|| path.with_extension(""));
        let (linked, link_symptoms) =
            native::link_executable(&obj_path, &exe_path, args.target.as_deref());
        if !linked {
            for s in &link_symptoms {
                eprintln!("Link error: [{}] {}", s.error_code(), s.message);
            }
        }
        let _ = std::fs::remove_file(&obj_path);
    } else {
        for s in &symptoms {
            eprintln!("Codegen warning: [{}] {}", s.error_code(), s.message);
        }
        println!("Compiled to {}", obj_path.display());
    }
}
