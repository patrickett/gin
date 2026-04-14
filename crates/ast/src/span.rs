//! Span types re-exported from the `span` crate.
//!
//! The canonical definitions of `Span`, `SpanId`, `SpanTable`, and `Spanned`
//! live in the `span` crate so that foundational crates (lexer, ast, diagnostic)
//! can share them without pulling in heavy transitive dependencies.

pub use span::{Span, SpanId, SpanTable, Spanned};
