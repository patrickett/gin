//! AST definitions and parsing utilities for the Gin compiler.

pub mod ast;
pub mod parse;

pub use ast::*;

// Re-export parse utilities
pub use parse::{ParserError, block, delimited_list, unescape};

pub use internment::Intern;
pub use lexer::{MAX_INDENT_DEPTH, Token};

/// Prelude for AST parsing
pub mod prelude {
    pub use crate::ast::*;
    pub use crate::parse::ParserError;
    pub use chumsky::{input::ValueInput, prelude::*};
    pub use internment::Intern;
    pub use lexer::{MAX_INDENT_DEPTH, Token};
}
