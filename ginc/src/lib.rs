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

pub use args::*;
pub use ast::{DefMap, FileAst, Symbol, SymbolKind, SymbolTable, TagMap};
pub use database::{
    File,
    input_database::{Db, InputDatabase},
};
pub use diagnostic::{Category, Symptom, SymptomSource};

use crate::ast::ImportSource;
use crate::compilation::{compile::compile_entry, native};
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
        let (tx, _rx) = unbounded();
        let db = InputDatabase::new(tx);

        let path = args.input.to_owned();

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
                let ast = load_entry_with_deps(&path, &args.dependencies);
                if let Err(e) = native::compile_to_object(&ast, &obj_path, args.profile) {
                    eprintln!("Codegen error: {e:?}");
                }
            }
            Emit::Exe => {
                let exe_path = args
                    .output
                    .clone()
                    .unwrap_or_else(|| path.with_extension(""));
                let obj_path = exe_path.with_extension("o");
                let ast = load_entry_with_deps(&path, &args.dependencies);
                match native::compile_to_object(&ast, &obj_path, args.profile) {
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

/// Parse the entry file and merge all matching flask.json dependencies into its AST.
fn load_entry_with_deps(entry_path: &Path, dependencies: &HashMap<String, PathBuf>) -> FileAst {
    let src = std::fs::read_to_string(entry_path).unwrap_or_default();
    let mut ast = parse_from_str(&src);

    if dependencies.is_empty() {
        return ast;
    }

    // Collect package import names that have a resolved dependency path.
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
            ast.merge_from(load_gin_dir(dep_dir));
        }
    }

    ast
}

/// Parse all `.gin` files in `dir` and merge them into a single FileAst.
fn load_gin_dir(dir: &Path) -> FileAst {
    let mut merged = FileAst::default();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().map_or_else(|| false, |e| e == "gin")
                && let Ok(src) = std::fs::read_to_string(&p)
            {
                merged.merge_from(parse_from_str(&src));
            }
        }
    }
    merged
}
