//! Frontend is everything the user interacts with. This is what handles the source code.

pub mod lexer;
pub use lexer::{GinLexer, IndentToken, Token};
pub mod parser;

pub mod prelude {
    pub use crate::frontend::lexer::*;
    pub use crate::frontend::parser::{ParserError, Spanned, construct::*};
    pub use chumsky::{input::ValueInput, prelude::*};
}
