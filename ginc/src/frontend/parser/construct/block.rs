use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*};

pub fn block<'tokens, 'src: 'tokens, I, Header, Closer, Body>(
    header: impl Parser<'tokens, I, Header, ParserError<'tokens, 'src>> + Clone,
    body_expr: impl Parser<'tokens, I, Body, ParserError<'tokens, 'src>> + Clone,
    closer: impl Parser<'tokens, I, Closer, ParserError<'tokens, 'src>> + Clone,
) -> impl Parser<'tokens, I, (Header, Vec<Body>, Closer), ParserError<'tokens, 'src>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    header
        .then(just(Token::Newline).repeated())
        .then(just(Token::Indent).or_not())
        .then(body_expr.clone().repeated().collect())
        .then(just(Token::Dedent).or_not())
        .then(closer.clone())
        .map(
            |(((((header, _newlines), _indent_opt), body), _dedent_opt), closer_val)| {
                (header, body, closer_val)
            },
        )
}
