mod args;
pub mod ast;
pub mod codegen;
pub mod compilation;
pub mod database;
pub mod diagnostic;
pub mod intern;
pub mod lexer;
pub mod parse;

pub use args::*;
pub use database::{
    File,
    input_database::{Db, InputDatabase},
};
pub use diagnostic::{Category, Symptom, SymptomSource};
pub use ast::{DefMap, FileAst, Symbol, SymbolKind, SymbolTable, TagMap};

use crate::compilation::compile::compile;
use crossbeam_channel::unbounded;

pub const GIN_FILE_EXT: &str = "gin";

pub mod prelude {
    pub use crate::ast::*;
    pub use crate::codegen::{CodegenContext, Lower, RuntimeSymbolTable};
    pub use crate::intern::IStr;
    pub use crate::lexer::{Token, MAX_INDENT_DEPTH};
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

        // Compile (this triggers parse → resolve_imports → codegen)
        let compiled = compile(&db, entry);

        // Collect and print all accumulated diagnostics
        let filename = path.to_string_lossy().to_string();
        let diagnostics = compile::accumulated::<Symptom>(&db, entry);
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
        if !bytecode.is_empty() {
            let mlir_text = String::from_utf8_lossy(bytecode);
            println!("\n```mlir\n{mlir_text}```\n");
        } else {
            eprintln!("Compilation failed or produced no output");
        }
    }
}
