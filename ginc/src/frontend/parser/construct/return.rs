use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*, span::SimpleSpan};

#[derive(Debug, Clone)]
pub struct Return(pub Option<Box<Expr>>);

pub fn r#return<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Return, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    just(Token::Return)
        .ignore_then(expr.or_not())
        .map(|e| Return(e.map(Box::new)))
}
