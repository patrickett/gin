mod args;
pub mod backend;
pub mod database;
pub mod diagnostic;
pub mod frontend;
pub mod source;
use crate::diagnostic::*;
use crate::frontend::parser::Parsable;
pub use args::*;

pub const GIN_FILE_EXT: &str = "gin";
pub const BINARY_ENTRY_FILE_NAME: &str = "main.gin";

pub type GincResult<'a, T> = Result<(GincWarnings, T), GincFlaw<'a>>;

/// Analagous to the `ginc` command
pub struct GinCompiler;

impl GinCompiler {
    pub fn compile(args: &'_ mut Args) {
        match &args.input.to_ast() {
            Ok(ast) => println!("{:#?}", ast),
            Err(_errors) => {
                // TODO: print errors
            }
        }
    }
}
