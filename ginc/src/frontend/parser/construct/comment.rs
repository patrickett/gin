use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*, span::SimpleSpan};

#[derive(Debug, Clone)]
pub struct Comment(pub String);

pub fn comment<'t, 's: 't, I>() -> impl Parser<'t, I, Comment, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    select! { Token::Comment(text) => text }.map(|c| Comment(c.to_string()))
}
