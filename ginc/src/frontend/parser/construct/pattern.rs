use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Pattern {
    Ident(IStr),
    Tuple(Vec<Pattern>),
    // etc.
}

pub fn pattern<'t, I>() -> impl Parser<'t, I, Pattern, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let id = select! { Token::Id(s) => Pattern::Ident(IStr::new(s.to_string())) };
    let tuple = id
        .repeated()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
        .map(Pattern::Tuple);

    choice((id, tuple))
}
