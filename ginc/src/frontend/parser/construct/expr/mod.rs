use crate::frontend::prelude::*;

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
pub use r#if::*;
pub mod range;
pub use range::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expr {
    Loop(Loop),
    If(IfExpr),
    Binary(Binary),
    FnCall(FnCall),
    Lit(Literal),
    FormatString(FormatString),
    Range(Range),
    Bind(Bind),
    Nothing,
}

/// Create a simple expression parser for for loop headers.
/// This includes range expressions and arithmetic, but not assignment or nested for loops.
fn for_loop_header_expr<'t, I>() -> impl Parser<'t, I, Expr, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;
    use chumsky::pratt::{infix, left};

    let atom = recursive(|expr| {
        choice((
            literal().boxed().map(Expr::Lit),
            fn_call(expr.clone()).boxed(),
        ))
    });

    // Range operator (precedence 2)
    let range = infix(left(2), just(Infer), |lhs: Expr, _, rhs: Expr, _| {
        Expr::Range(Range {
            start: Box::new(lhs),
            end: Box::new(rhs),
        })
    });

    // Arithmetic operators (precedence 3)
    let arithmetic = infix(
        left(3),
        select! {
            Plus => BinOp::Add,
            Minus => BinOp::Subtract,
            Star => BinOp::Multiply,
            Slash => BinOp::Divide,
        },
        |lhs: Expr, op: BinOp, rhs: Expr, _| {
            Expr::Binary(Binary {
                lhs: Box::new(lhs),
                op,
                rhs: Box::new(rhs),
            })
        },
    );

    atom.pratt((range, arithmetic))
        .padded_by(just(Newline).repeated())
}

pub fn expression<'t, I>() -> impl Parser<'t, I, Expr, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    recursive(|expr| {
        use Token::*;
        use chumsky::pratt::{infix, left};

        let atom = choice((
            literal().boxed().map(Expr::Lit),
            format_string().boxed().map(Expr::FormatString),
            bind(expression_atom_inner(expr.clone())).map(Expr::Bind),
            fn_call(expression_atom_inner(expr.clone())).boxed(),
            for_in_loop(for_loop_header_expr(), expr.clone())
                .boxed()
                .map(|for_loop| Expr::Loop(LoopEnum::ForIn(for_loop))),
        ));

        // Comparison operators (precedence 3)
        let comparison = infix(
            left(3),
            comparison_op(),
            |lhs: Expr, op: BinOp, rhs: Expr, _| {
                Expr::Binary(Binary {
                    lhs: Box::new(lhs),
                    op,
                    rhs: Box::new(rhs),
                })
            },
        );

        // Arithmetic operators (precedence 4)
        let arithmetic = infix(
            left(4),
            arithmetic_op(),
            |lhs: Expr, op: BinOp, rhs: Expr, _| {
                Expr::Binary(Binary {
                    lhs: Box::new(lhs),
                    op,
                    rhs: Box::new(rhs),
                })
            },
        );

        // Build the Pratt parser
        atom.pratt((comparison, arithmetic))
            .padded_by(just(Newline).repeated()) // ignore newlines around everything
    })
}

/// Helper for expression atom that takes a recursive parser as parameter
fn expression_atom_inner<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    choice((
        literal().boxed().map(Expr::Lit),
        format_string().boxed().map(Expr::FormatString),
        fn_call(expr.clone()).boxed(),
        bind(expr.clone()).map(Expr::Bind),
        // Note: for loops are NOT included in expression_atom to avoid infinite recursion
    ))
}
