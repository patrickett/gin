#![deny(unsafe_code)]
#![warn(clippy::correctness, clippy::suspicious, clippy::style, clippy::complexity, clippy::perf)]
//! Lexer for the Gin programming language.

mod debug;
mod handwritten;
mod token;

pub use debug::{TokenSpanned, debug_tokens};
pub use handwritten::Lexer;
pub use token::{LexContext, MAX_INDENT_DEPTH, Token};

// TODO: move this to whatever is referencing it
/// Returns true if `byte_pos` falls inside a comment token in `source`.
///
/// More accurate than scanning for `--` in source text, which would false-positive
/// on `--` inside string literals.
pub fn is_comment_at(source: &str, byte_pos: usize) -> bool {
    let mut lexer = Lexer::new(source);
    while let Some((tok, span_id)) = lexer.next_raw() {
        let span = lexer.get_span(span_id);
        if matches!(tok, Token::Comment(_)) && span.contains(byte_pos) {
            return true;
        }
    }
    false
}
