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

/// Create a simple expression parser for for loop headers.
/// This includes range expressions and arithmetic, but not assignment or nested for loops.
fn for_loop_header_expr<'t, 's: 't, I>() -> impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    use Token::*;
    use chumsky::pratt::{infix, left};

    let atom = recursive(|expr| {
        choice((
            literal().boxed().map(Expr::Lit),
            fn_call(expr.clone()).boxed(),
            bind(expr.clone()).map(Expr::Bind),
        ))
    });

    // Range operator (precedence 2)
    let range = infix(left(2), just(DotDot), |lhs: Expr, _, rhs: Expr, _| {
        Expr::Binary(Binary {
            lhs: Box::new(lhs),
            op: BinOp::Range,
            rhs: Box::new(rhs),
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
        .padded_by(comments())
}

pub fn expression<'t, 's: 't, I>() -> impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    recursive(|expr| {
        use Token::*;
        use chumsky::pratt::{infix, left};

        let atom = choice((
            literal().boxed().map(Expr::Lit),
            fn_call(expression_atom_inner(expr.clone())).boxed(),
            bind(expression_atom_inner(expr.clone())).map(Expr::Bind),
            for_in_loop(for_loop_header_expr(), expr.clone())
                .boxed()
                .map(ControlFlow::ForIn)
                .map(Expr::CtrlFlow),
        ));

        // Assignment has lowest precedence (1) - uses Colon token
        let assignment = infix(left(1), just(Colon), |lhs: Expr, _, rhs: Expr, _| {
            Expr::Binary(Binary {
                lhs: Box::new(lhs),
                op: BinOp::Assign,
                rhs: Box::new(rhs),
            })
        });

        // Range operator (precedence 2) - higher than comparison, lower than arithmetic
        let range = infix(left(2), just(DotDot), |lhs: Expr, _, rhs: Expr, _| {
            Expr::Binary(Binary {
                lhs: Box::new(lhs),
                op: BinOp::Range,
                rhs: Box::new(rhs),
            })
        });

        // Comparison operators (precedence 3)
        let comparison = infix(
            left(3),
            select! {
                Equals => BinOp::Equal,
                NotEqual => BinOp::NotEqual,
                Less => BinOp::LessThan,
                Greater => BinOp::GreaterThan,
                LessEq => BinOp::LessThanOrEqual,
                GreaterEq => BinOp::GreaterThanOrEqual,
            },
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

        // Build the Pratt parser
        atom.pratt((assignment, comparison, range, arithmetic))
            .padded_by(just(Newline).repeated()) // ignore newlines around everything
            .padded_by(comments())
    })
}

/// Helper for expression atom that takes a recursive parser as parameter
fn expression_atom_inner<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    choice((
        literal().boxed().map(Expr::Lit),
        fn_call(expr.clone()).boxed(),
        bind(expr.clone()).map(Expr::Bind),
        // Note: for loops are NOT included in expression_atom to avoid infinite recursion
    ))
}
