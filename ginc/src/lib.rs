mod args;
pub mod ast;
pub mod codegen;
pub mod compilation;
pub mod database;
pub mod diagnostic;
pub mod intern;
pub mod lexer;
pub mod parse;
pub mod typeck;

use crate::typeck::TyEnv;
pub use args::*;
pub use ast::{DefMap, FileAst, Symbol, SymbolKind, SymbolTable, TagMap};
pub use compilation::hover::find_definition_span;
pub use database::{
    File,
    input_database::{Db, InputDatabase},
};
pub use diagnostic::{Category, Symptom, SymptomSource};

use crate::ast::ImportSource;
use crate::compilation::{compile::compile_entry, native};
use crate::parse::parse::parse as salsa_parse;
use crate::parse::parse_from_str;
use crossbeam_channel::unbounded;
use std::collections::HashMap;
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
    pub fn compile(args: &'_ mut Args) {
        let path = args.input.to_owned();

        // For library builds (directory input), use a simpler compilation path
        if path.is_dir() {
            compile_library(args, &path);
            return;
        }

        // Regular build: use Salsa database for incremental compilation
        let (tx, _rx) = unbounded();
        let db = InputDatabase::new(tx);

        let entry = match db.input(path.clone()) {
            Ok(file) => file,
            Err(err) => {
                eprintln!("Error: {}", err);
                return;
            }
        };

        // Read the source file for error reporting
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("Error reading file: {}", err);
                return;
            }
        };

        // Compile entry point and all its imports in parallel.
        let compiled = compile_entry(&db, entry);

        // Collect diagnostics from the full dependency graph.
        let filename = path.to_string_lossy().to_string();
        let diagnostics = compile_entry::accumulated::<Symptom>(&db, entry);
        let has_flaws = diagnostics
            .iter()
            .any(|d| matches!(d.category, diagnostic::Category::Flaw));

        for diagnostic in &diagnostics {
            diagnostic.print(&source, &filename);
        }

        if has_flaws {
            return;
        }

        let bytecode = compiled.bytecode(&db);
        if bytecode.is_empty() {
            eprintln!("Compilation failed or produced no output");
            return;
        }
        let mlir_text = String::from_utf8_lossy(bytecode);

        match args.emit {
            Emit::Mlir => {
                println!("\n```mlir\n{mlir_text}```\n");
            }
            Emit::Obj => {
                let obj_path = args
                    .output
                    .clone()
                    .unwrap_or_else(|| path.with_extension("o"));
                let ast = build_native_ast(&db, entry, &args.dependencies);
                let ty_env = TyEnv::from_file_ast(&ast);
                if let Err(e) = native::compile_to_object(
                    &ast,
                    &obj_path,
                    args.profile,
                    &source,
                    &filename,
                    &ty_env,
                ) {
                    eprintln!("Codegen error: {e:?}");
                }
            }
            Emit::Exe => {
                let exe_path = args
                    .output
                    .clone()
                    .unwrap_or_else(|| path.with_extension(""));
                let obj_path = exe_path.with_extension("o");
                let ast = build_native_ast(&db, entry, &args.dependencies);
                let ty_env = TyEnv::from_file_ast(&ast);
                match native::compile_to_object(
                    &ast,
                    &obj_path,
                    args.profile,
                    &source,
                    &filename,
                    &ty_env,
                ) {
                    Err(e) => eprintln!("Codegen error: {e:?}"),
                    Ok(()) => {
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
    }
}

/// Compile a library directory (all .gin files) to an object file.
///
/// Unlike the binary path which follows imports from a single entry point,
/// library builds gather ALL `.gin` files in the directory, parse them through
/// the Salsa pipeline for proper diagnostic gathering, and only proceed to
/// codegen if there are no parse/lex errors.
fn compile_library(args: &Args, path: &Path) {
    // Collect all .gin file paths in the directory
    let gin_files = collect_gin_files_recursive(path);

    if gin_files.is_empty() {
        eprintln!("No .gin files found in {}", path.display());
        return;
    }

    // Create Salsa database for proper diagnostic gathering
    let (tx, _rx) = unbounded();
    let db = InputDatabase::new(tx);

    // Load all files into the database
    let mut files: Vec<File> = Vec::new();
    let mut filenames: Vec<String> = Vec::new();

    for file_path in &gin_files {
        let filename = file_path.to_string_lossy().into_owned();
        match db.input(file_path.clone()) {
            Ok(file) => {
                files.push(file);
                filenames.push(filename);
            }
            Err(err) => {
                eprintln!("Error loading {}: {}", file_path.display(), err);
                return;
            }
        }
    }

    // Parse each file through the Salsa pipeline (accumulates parse/lex diagnostics)
    // and build the merged AST at the same time — Salsa caches the parse result so
    // the second call per file below is a free cache hit.
    use crate::parse::parse::parse as salsa_parse;
    let mut ast = FileAst::default();
    for &file in &files {
        ast.merge_from(salsa_parse(&db, file));
    }

    // Collect and print parse diagnostics per-file with correct source context
    let mut has_flaws = false;
    for (i, &file) in files.iter().enumerate() {
        let source = file.contents(&db);
        let filename = &filenames[i];
        let diagnostics = salsa_parse::accumulated::<Symptom>(&db, file);
        for diagnostic in &diagnostics {
            diagnostic.print(source, filename);
            if matches!(diagnostic.category, Category::Flaw) {
                has_flaws = true;
            }
        }
    }

    if has_flaws {
        return;
    }

    // Determine output path
    let obj_path = args.output.clone().unwrap_or_else(|| {
        let pkg_name = path.file_name().unwrap_or_default().to_string_lossy();
        path.join("target").join(format!("{}.o", pkg_name))
    });

    // Ensure target directory exists
    if let Some(parent) = obj_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let dir_name = path.to_string_lossy().into_owned();
    let ty_env = TyEnv::from_file_ast(&ast);
    match native::compile_to_object(&ast, &obj_path, args.profile, "", &dir_name, &ty_env) {
        Ok(()) => {
            println!("Compiled library to {}", obj_path.display());
        }
        Err(e) => {
            eprintln!("Codegen error: {e:?}");
        }
    }
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

/// Recursively load all .gin files in a directory
fn load_gin_dir_recursive(dir: &Path) -> FileAst {
    let mut merged = FileAst::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return merged;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip target directory (build artifacts)
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            merged.merge_from(load_gin_dir_recursive(&path));
        } else if path.extension().is_some_and(|ext| ext == "gin")
            && let Ok(src) = std::fs::read_to_string(&path)
        {
            merged.merge_from(parse_from_str(&src));
        }
    }
    merged
}

/// Build the AST for native compilation from the Salsa-cached entry parse result,
/// then merge in any flask.json package dependencies.
///
/// This replaces the old `load_entry_with_deps` which re-read the entry file from
/// disk and re-parsed it. Now the entry AST is a cache hit from the Salsa pipeline
/// that already ran for diagnostics. Package deps (not tracked by Salsa) are still
/// loaded from disk.
fn build_native_ast(db: &dyn Db, entry: File, dependencies: &HashMap<String, PathBuf>) -> FileAst {
    let mut ast = salsa_parse(db, entry);

    if dependencies.is_empty() {
        return ast;
    }

    let dep_names: Vec<String> = ast
        .uses()
        .iter()
        .flat_map(|imp| &imp.0)
        .filter_map(|mi| {
            if let ImportSource::Package(path) = &mi.source {
                let name = path.root.to_string();
                if dependencies.contains_key(&name) {
                    return Some(name);
                }
            }
            None
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    for dep_name in dep_names {
        if let Some(dep_dir) = dependencies.get(&dep_name) {
            ast.merge_from(load_gin_dir_recursive(dep_dir));
        }
    }

    ast
}
