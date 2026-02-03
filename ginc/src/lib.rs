mod args;
pub mod backend;
pub mod database;
pub mod diagnostic;
pub mod frontend;
pub mod source;
use crate::database::{
    accumulator::Diagnostic,
    input_database::{Db, InputDatabase},
};
use crate::diagnostic::*;
pub use args::*;
use crossbeam_channel::unbounded;

pub const GIN_FILE_EXT: &str = "gin";
pub const BINARY_ENTRY_FILE_NAME: &str = "main.gin";

pub type GincResult<'a, T> = Result<(GincWarnings, T), GincFlaw<'a>>;

/// Analagous to the `ginc` command
pub struct GinCompiler;

impl GinCompiler {
    pub fn compile(args: &'_ mut Args) {
        let (tx, _rx) = unbounded();
        let db = InputDatabase::new(tx);

        let path = args.input.to_owned();

        let entry = match db.input(path) {
            Ok(file) => file,
            Err(err) => {
                eprintln!("Error: {}", err);
                return;
            }
        };

        // Use salsa query instead of Parsable::to_ast()
        let (_ast, _deps) = database::parse(&db, entry);

        // Report diagnostics from accumulator
        let diagnostics = database::parse_dependencies::accumulated::<Diagnostic>(&db, entry);
        for diagnostic in diagnostics {
            eprintln!("{}", diagnostic.0);
        }

        // Compile to MLIR
        let compiled = database::compile(&db, entry);
        let bytecode = compiled.bytecode(&db);

        if !bytecode.is_empty() {
            // Print the MLIR for debugging
            let mlir_text = String::from_utf8_lossy(bytecode);
            println!("\n```mlir\n{mlir_text}```\n");
        } else {
            eprintln!("Compilation failed or produced no output");
        }
    }
}
