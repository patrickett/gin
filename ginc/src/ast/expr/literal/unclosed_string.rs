use crate::parse::unescape::unescape;
use crate::prelude::*;

pub fn unclosed_string<'t, I>() -> impl Parser<'t, I, Literal, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! { Token::UnterminatedString(s) => Literal::String(unescape(s)) }
}
