//! AST definitions and parsing utilities for the Gin compiler.

pub mod ast;
pub mod parse;
pub mod signature;
pub mod interface_hash;

pub use ast::*;
pub use signature::*;

// Re-export parse utilities
pub use parse::{ParserError, block, delimited_list, unescape};
pub use parse::query::{parse_from_str, resolve_imports, ast_hash};
pub use parse::query::parse as parse_file;

// Re-export interface hash
pub use interface_hash::{FileInterfaceHash, file_interface_hash};

/// Prelude for AST parsing
pub mod prelude {
    pub use crate::ast::*;
    pub use crate::parse::ParserError;
    pub use chumsky::{input::ValueInput, prelude::*};
    pub use lexer::Token;
    pub use internment::Intern;
}
