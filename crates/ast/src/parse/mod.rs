//! Parsing utilities for the Gin compiler.

pub mod block;
pub mod delimited_list;
pub mod unescape;
pub mod query;

pub use block::*;
pub use delimited_list::*;
pub use unescape::*;

use lexer::Token;
use chumsky::{extra, prelude::Rich};

/// Parser error type for chumsky parsers.
pub type ParserError<'t> = extra::Err<Rich<'t, Token<'t>>>;
