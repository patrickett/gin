use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binary {
    pub lhs: Box<Expr>,
    pub op: BinOp,
    pub rhs: Box<Expr>,
}

/// Binary operations are defined as `lhs op rhs`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BinOp {
    /// <=
    LessThanOrEqual,
    /// >=
    GreaterThanOrEqual,
    /// <
    LessThan,
    /// >
    GreaterThan,
    /// +
    Add,
    /// /
    Divide,
    /// *
    Multiply,
    /// -
    Subtract,
    /// :
    Assign,
    /// !=
    NotEqual,
    /// =
    Equal,
    /// ..
    Range,
}

pub fn binary_expr<'t, I, P>(expr: P) -> impl Parser<'t, I, Binary, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
    P: Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
{
    use Token::*;

    let op = select! {
        Plus => BinOp::Add,
        Minus => BinOp::Subtract,
        Star => BinOp::Multiply,
        Slash => BinOp::Divide,
        Colon => BinOp::Assign,
        // Assignment => BinOp::Assign,
    };

    expr.clone()
        .then(op.then(expr))
        .map(|(lhs, (op, rhs))| Binary {
            lhs: Box::new(lhs),
            op,
            rhs: Box::new(rhs),
        })
}
