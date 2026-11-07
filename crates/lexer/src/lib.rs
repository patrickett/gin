//! Allocation-free raw source scanning: each token is a classification and byte span.
//!
//! Recover spelling with `token.span.extract(original_source)`. Numeric conversion,
//! decoding, diagnostics, layout, and format grammar belong to consumers. Comments,
//! newlines, unsupported Unicode scalars, and malformed literals remain visible.

mod handwritten;
mod token;

pub use handwritten::Lexer;
pub use token::{Token, TokenKind};
