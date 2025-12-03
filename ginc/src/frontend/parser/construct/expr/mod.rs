use crate::frontend::prelude::*;

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
pub use binary::*;
mod for_loop;
pub use for_loop::*;

#[derive(Debug, Clone)]
pub enum Expr {
    CtrlFlow(ControlFlow),
    Binary(Binary),
    FnCall(FnCall),
    Lit(Literal),

    Bind(Bind),
    Nothing,
}

pub fn expression<'t, 's: 't, I>() -> impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    use Token::*;
    recursive(|expr| {
        choice((
            bind(expr.clone()).map(Expr::Bind),
            literal().boxed().map(Expr::Lit),
            for_in_loop(expr.clone())
                .boxed()
                .map(ControlFlow::ForIn)
                .map(Expr::CtrlFlow),
            // binary_expr(expr.clone()),
            // if_expr(expr.clone()),
            // for_in(expr.clone()),
            fn_call(expr.clone()).boxed(),
        ))
    })
    .padded_by(just(Newline).repeated()) // ignore newlines around everything
    .padded_by(comments())
}
