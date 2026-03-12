//! Frontend is everything the user interacts with. This is what handles the source code.

pub mod lexer;
pub use lexer::{GinLexer, Token};
pub mod parser;

pub mod prelude {
    pub use crate::frontend::lexer::{Token, MAX_INDENT_DEPTH};
    pub use crate::frontend::parser::{construct::*, delimited_list::*, ParserError};
    pub use crate::intern::IStr;
    pub use chumsky::{input::ValueInput, prelude::*};
}
