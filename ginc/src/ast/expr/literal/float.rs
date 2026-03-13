use crate::prelude::*;

pub fn float<'t, I>() -> impl Parser<'t, I, Literal, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! { Token::Float(f) => Literal::Float(f) }
}
