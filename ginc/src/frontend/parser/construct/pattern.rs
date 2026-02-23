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
    let id = id_token().map(Pattern::Ident);
    let tuple =
        delimited_list(Token::ParenOpen, id.clone(), Token::Comma, Token::ParenClose).map(Pattern::Tuple);

    choice((id, tuple))
}
