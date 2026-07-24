//! Parser for the Gin language.
//!
//! This crate contains the recursive-descent parser and content hashing.

pub mod content_hash;
pub mod cursor;
pub mod declare;
pub mod expr;
pub mod gin_walk;
mod int_range;
pub mod module;
pub mod params;
pub mod path;
pub mod query;
pub mod tag;
mod top_level;
pub mod type_annotation;
pub mod unescape;

use ast::FileAst;

/// Convenience function to parse Gin source text directly.
///
/// This is the primary entry point used by external consumers.
/// It lexes the input, filters comments, runs the handwritten parser,
/// and attaches lex errors as `parse_warnings` on the AST.
pub fn parse_from_str(source: &str) -> FileAst {
    cursor::TokenCursor::parse_source(source)
}
