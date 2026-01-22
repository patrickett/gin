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
    use chumsky::pratt::{infix, left};

    let atom = choice((
        literal().boxed().map(Expr::Lit),
        fn_call(expression_atom()).boxed(),
        bind(expression_atom()).map(Expr::Bind),
        for_in_loop(expression_atom())
            .boxed()
            .map(ControlFlow::ForIn)
            .map(Expr::CtrlFlow),
    ));

    // Assignment has lowest precedence (1)
    let assignment = infix(left(1), just(Assignment), |lhs: Expr, _, rhs: Expr, _| {
        Expr::Binary(Binary {
            lhs: Box::new(lhs),
            op: BinOp::Assign,
            rhs: Box::new(rhs),
        })
    });

    // Comparison operators (precedence 2)
    let comparison = infix(
        left(2),
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

    // Build the Pratt parser
    atom.pratt((assignment, comparison, arithmetic))
        .padded_by(just(Newline).repeated()) // ignore newlines around everything
        .padded_by(comments())
}

// prevent infinite recursion in atomic elements
fn expression_atom<'t, 's: 't, I>() -> impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    recursive(|expr| {
        choice((
            literal().boxed().map(Expr::Lit),
            fn_call(expr.clone()).boxed(),
            bind(expr.clone()).map(Expr::Bind),
            for_in_loop(expr)
                .boxed()
                .map(ControlFlow::ForIn)
                .map(Expr::CtrlFlow),
        ))
    })
}
