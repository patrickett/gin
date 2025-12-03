use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum Pattern {
    Ident(String),
    Tuple(Vec<Pattern>),
    // etc.
}

pub fn pattern<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Pattern, ParserError<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let id = select! {Token::Id(s) => s.to_string()}.map(Pattern::Ident);
    let tuple = id
        .repeated()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
        .map(Pattern::Tuple);

    choice((id, tuple))
}
