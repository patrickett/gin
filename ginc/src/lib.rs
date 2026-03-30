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
pub use lexer::semantic_tokens::{
    RawSemanticToken, TOKEN_FUNCTION, TOKEN_METHOD, semantic_tokens_raw,
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

                // Pipe the MLIR text already produced by the Salsa pipeline
                // directly to native compilation instead of regenerating it.
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
