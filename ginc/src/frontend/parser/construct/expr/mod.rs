mod bind;
pub use bind::*;
mod control_flow;
pub use control_flow::*;
mod literal;
pub use literal::*;
mod import;
pub use import::*;
mod fn_call;
pub use fn_call::*;
mod binary;
use crate::frontend::prelude::*;
pub use binary::*;

#[derive(Debug, Clone)]
pub enum Expr {
    CtrlFlow(ControlFlow),
    Binary(Binary),
    FnCall(FnCall),
    Lit(Literal),

    Expr(Bind),
    Nothing,
}

pub fn expression<'t, 's: 't, I>() -> impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    recursive(|expr| {
        choice((
            bind(expr.clone()).map(Expr::Expr),
            literal().boxed().map(Expr::Lit),
            // binary_expr(expr.clone()),
            // if_expr(expr.clone()),
            // for_in(expr.clone()),
            fn_call(expr.clone()).boxed(),
        ))
    })
    .padded_by(just(Token::Newline).repeated()) // ignore newlines around everything
    .padded_by(comment().repeated())
}
