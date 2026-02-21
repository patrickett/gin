use crate::frontend::prelude::*;
use chumsky::{input::ValueInput, prelude::*, Parser};

pub fn block<'t, I, Header, Closer, Body>(
    header: impl Parser<'t, I, Header, ParserError<'t>> + Clone,
    body_expr: impl Parser<'t, I, Body, ParserError<'t>> + Clone,
    closer: impl Parser<'t, I, Closer, ParserError<'t>> + Clone,
) -> impl Parser<'t, I, (Header, Vec<Body>, Closer), ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
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
