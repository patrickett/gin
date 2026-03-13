use crate::prelude::*;

mod bind;
pub use bind::*;
pub mod format_string;
pub use format_string::*;
pub mod literal;
pub use literal::*;
mod import;
pub use import::*;
mod fn_call;
pub use fn_call::*;
mod binary;
pub use binary::*;
pub mod r#loop;
pub use r#loop::{Loop as LoopEnum, *};
pub mod r#if;
pub mod range;
pub use range::*;
pub mod r#return;
pub use r#return::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expr {
    Loop(Loop),
    Binary(Binary),
    FnCall(FnCall),
    Lit(Literal),
    FormatString(FormatString),
    Range(Range),
    Bind(Bind),
    Nothing,
}

pub fn expression<'t, I>() -> impl Parser<'t, I, Expr, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    recursive(|expr| {
        use chumsky::pratt::{infix, left};
        use Token::*;

        let inner = atom(expr.clone());

        let full_atom = choice((
            inner.boxed(),
            loop_expr(expr.clone()).map(Expr::Loop).boxed(),
        ));

        // Comparison operators (precedence 3)
        let comparison = infix(
            left(3),
            comparison_op(),
            |lhs: Expr, op: BinOp, rhs: Expr, _| Expr::Binary(Binary::new(lhs, op, rhs)),
        );

        // Arithmetic operators (precedence 4)
        let arithmetic = infix(
            left(4),
            arithmetic_op(),
            |lhs: Expr, op: BinOp, rhs: Expr, _| Expr::Binary(Binary::new(lhs, op, rhs)),
        );

        full_atom
            .pratt((comparison, arithmetic))
            .padded_by(just(Newline).repeated())
    })
}

/// Base expression atoms — literals, format strings, function calls, and binds.
///
/// Does NOT include loops to prevent infinite recursion when used as the
/// sub-expression parser for bind and fn_call.
fn atom<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    choice((
        literal().map(Expr::Lit).boxed(),
        format_string(expr.clone()).map(Expr::FormatString).boxed(),
        fn_call(expr.clone()).map(Expr::FnCall).boxed(),
        bind(expr.clone()).map(Expr::Bind).boxed(),
    ))
}
