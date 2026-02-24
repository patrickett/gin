//! Frontend is everything the user interacts with. This is what handles the source code.

pub mod lexer;
pub use lexer::{GinLexer, Token};
pub mod parser;

pub mod prelude {
    pub use crate::frontend::lexer::{MAX_INDENT_DEPTH, Token};
    pub use crate::frontend::parser::{ParserError, Spanned, construct::*, delimited_list::*};
    pub use crate::intern::IStr;
    pub use chumsky::{input::ValueInput, prelude::*};
}
