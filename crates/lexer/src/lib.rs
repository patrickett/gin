//! Lexer for the Gin programming language.

mod handwritten;
mod token;

pub use handwritten::Lexer;
pub use token::{MAX_INDENT_DEPTH, Token};
