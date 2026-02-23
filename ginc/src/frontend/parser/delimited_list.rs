use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*};

/// Parse a delimited, separated list like `(a, b, c)`.
pub fn delimited_list<'t, I, T>(
    open: Token<'t>,
    element: impl Parser<'t, I, T, ParserError<'t>> + Clone,
    separator: Token<'t>,
    close: Token<'t>,
) -> impl Parser<'t, I, Vec<T>, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    element
        .separated_by(just(separator))
        .collect::<Vec<_>>()
        .delimited_by(just(open), just(close))
}
