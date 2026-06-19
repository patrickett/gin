//! Lexer for the Gin programming language.

mod debug;
mod handwritten;
mod token;

pub use debug::DebugTokensExt;
pub use handwritten::Lexer;
pub use token::{MAX_INDENT_DEPTH, Token};
