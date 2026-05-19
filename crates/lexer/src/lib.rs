#![deny(unsafe_code)]
#![warn(
    clippy::correctness,
    clippy::suspicious,
    clippy::style,
    clippy::complexity,
    clippy::perf
)]
//! Lexer for the Gin programming language.

mod debug;
mod handwritten;
mod token;

pub use debug::debug_tokens;
pub use handwritten::Lexer;
pub use token::{MAX_INDENT_DEPTH, Token};
