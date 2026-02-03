use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub struct Binary {
    pub lhs: Box<Expr>,
    pub op: BinOp,
    pub rhs: Box<Expr>,
}

/// Binary operations are defined as `lhs op rhs`
#[derive(Debug, Clone)]
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

pub fn binary_expr<'t, 's: 't, I, P>(expr: P) -> impl Parser<'t, I, Binary, ParserError<'t, 's>>
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
    P: Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
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
