//! Frontend is everything the user interacts with. This is what handles the source code.

pub mod lexer;
pub use lexer::{GinLexer, InternedToken, Token};
pub mod parser;

pub mod prelude {
    pub use crate::frontend::lexer::{InternedToken, Token, MAX_INDENT_DEPTH};
    pub use crate::frontend::parser::{construct::*, ParserError, Spanned};
    pub use crate::intern::IStr;
    pub use chumsky::{input::ValueInput, prelude::*};
}
