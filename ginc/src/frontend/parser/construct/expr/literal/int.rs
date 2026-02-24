use crate::frontend::prelude::*;

pub fn int<'t, I>() -> impl Parser<'t, I, Literal, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! { Token::Int(n) => Literal::Int(n) }
}
