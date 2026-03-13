//! Parsing utilities and parser implementation.

pub mod block;
pub mod delimited_list;
pub mod parse;
pub mod unescape;

pub use block::*;
pub use delimited_list::*;
pub use parse::*;
pub use unescape::*;

use crate::lexer::Token;
use chumsky::{extra, prelude::Rich};

/// Parser error type for chumsky parsers.
pub type ParserError<'t> = extra::Err<Rich<'t, Token<'t>>>;
