use crate::prelude::*;

/// Parse an identifier token into an interned string.
pub fn id_token<'t, I>() -> impl Parser<'t, I, IStr, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! { Token::Id(name) => IStr::new(name.to_string()) }
}
