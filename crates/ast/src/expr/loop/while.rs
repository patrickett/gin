use crate::block;
use crate::prelude::*;

/// While loop: loop while a condition holds.
///
/// ```gin
/// main:
///     while x < 10
///     loop
/// return
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhileLoop {
    pub cond: Box<Spanned<Expr>>,
    pub exprs: Vec<Spanned<Expr>>,
}

pub fn while_loop<'t, I>(
    body_expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, WhileLoop, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    block(
        just(Token::While).ignore_then(body_expr.clone()),
        body_expr,
        just(Token::Loop),
    )
    .map(|(cond, exprs, _)| WhileLoop {
        cond: Box::new(cond),
        exprs,
    })
}
