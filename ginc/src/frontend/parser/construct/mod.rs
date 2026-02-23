use crate::frontend::prelude::*;

pub mod expr;
pub use expr::*;
mod ast;
pub use ast::*;

/// Parse an identifier token into an interned string.
pub fn id_token<'t, I>() -> impl Parser<'t, I, IStr, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! { Token::Id(name) => IStr::new(name.to_string()) }
}

// sub constructs that are nothing by themself
mod parameter;
pub use parameter::*;
mod path;
pub use path::*;
mod tag;
pub use tag::*;
mod doc_comment;
pub use doc_comment::*;
mod declare;
pub use declare::*;
mod pattern;
pub use pattern::*;
