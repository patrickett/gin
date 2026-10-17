//! Parser for the Gin language.
//!
//! This crate contains the recursive-descent parser and content hashing.
#[cfg(test)]
extern crate self as parser;

pub mod cursor;
mod declaration_source;
pub mod declare;
pub mod expr;
mod int_range;
mod lowering;
pub mod params;
pub mod path;
pub mod query;
pub mod tag;
mod top_level;
pub mod type_annotation;
pub mod unescape;

use ast::FileAst;
pub use declaration_source::{SourceDeclaration, source_declarations};

/// Convenience function to parse Gin source text directly.
///
/// This is the primary entry point used by external consumers.
/// It lexes the input, filters comments, runs the handwritten parser,
/// canonicalizes parser method keys via post-parse lowering, and attaches lex
/// errors as `parse_warnings` on the AST.
pub fn parse_from_str(source: &str) -> FileAst {
    cursor::TokenCursor::parse_source(source)
}
