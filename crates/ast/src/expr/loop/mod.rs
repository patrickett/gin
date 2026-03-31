use crate::prelude::*;

pub mod r#for;
pub use r#for::*;

pub mod r#while;
pub use r#while::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Loop {
    While(WhileLoop),
    ForIn(ForInLoop),
}

pub fn loop_expr<'t, I>(
    body_expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Loop, ParserError<'t>> + Clone + 't
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    choice((
        while_loop(body_expr.clone()).map(Loop::While),
        for_in_loop(for_loop_header_expr(), body_expr).map(Loop::ForIn),
    ))
    .boxed()
}
