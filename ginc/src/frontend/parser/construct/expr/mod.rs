mod assignment;
pub use assignment::*;
mod control_flow;
pub use control_flow::*;
mod literal;
pub use literal::*;
mod r#use;
pub use r#use::*;
mod fn_call;
pub use fn_call::*;
mod binary;
pub use binary::*;

use crate::frontend::prelude::*;

// TODO: convert some expr to TopLevelExpr
// ctrlflow,binary,fn_call, literal has to be inside a fn
// tl: use, assign, comment

#[derive(Debug, Clone)]
pub enum Expr<'src> {
    // Use(UseExpr<'src>),
    CtrlFlow(ControlFlow<'src>),
    Lit(Literal),
    FnCall(FnCall<'src>),
    Assignment(Assignment<'src>),
    Binary(Binary<'src>),
    Comment(&'src str),
    Nothing,
}

pub fn expression<'t, 's: 't, I>() -> impl Parser<'t, I, Expr<'s>, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    recursive(|expr| {
        choice((
            assignment(expr.clone(), tag(expr.clone())).map(Expr::Assignment),
            literal().boxed().map(Expr::Lit),
            // binary_expr(expr.clone()),
            // if_expr(expr.clone()),
            // for_in(expr.clone()),
            fn_call(expr.clone()).boxed(),
        ))
    })
    .padded_by(just(Token::Newline).repeated()) // ignore newlines around everything
}
