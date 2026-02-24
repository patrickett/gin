use crate::frontend::parser::unescape::unescape;
use crate::frontend::prelude::*;

pub fn string<'t, I>() -> impl Parser<'t, I, Literal, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! { Token::String(s) => Literal::String(unescape(s)) }
}
