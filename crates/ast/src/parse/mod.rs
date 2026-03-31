//! Parsing utilities for the Gin compiler.

pub mod query;
pub mod unescape;

pub use unescape::*;

use chumsky::{extra, prelude::Rich};
use lexer::Token;

/// Parser error type for chumsky parsers.
pub type ParserError<'t> = extra::Err<Rich<'t, Token<'t>>>;
