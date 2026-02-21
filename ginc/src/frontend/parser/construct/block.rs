use crate::frontend::prelude::*;
use chumsky::{input::ValueInput, prelude::*, Parser};

pub fn block<'tokens, I, Header, Closer, Body>(
    header: impl Parser<'tokens, I, Header, ParserError<'tokens>> + Clone,
    body_expr: impl Parser<'tokens, I, Body, ParserError<'tokens>> + Clone,
    closer: impl Parser<'tokens, I, Closer, ParserError<'tokens>> + Clone,
) -> impl Parser<'tokens, I, (Header, Vec<Body>, Closer), ParserError<'tokens>> + Clone
where
    I: ValueInput<'tokens, Token = Token, Span = SimpleSpan>,
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
