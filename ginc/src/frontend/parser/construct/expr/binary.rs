use crate::frontend::prelude::*;
use chumsky::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binary {
    pub lhs: Box<Expr>,
    pub op: BinOp,
    pub rhs: Box<Expr>,
}

impl Binary {
    pub fn new(lhs: Expr, op: BinOp, rhs: Expr) -> Self {
        let lhs = Box::new(lhs);
        let rhs = Box::new(rhs);
        Self { lhs, op, rhs }
    }
}

/// Binary operations are defined as `lhs op rhs`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BinOp {
    LessThanOrEqual,
    GreaterThanOrEqual,
    LessThan,
    GreaterThan,
    Add,
    Divide,
    Multiply,
    Subtract,
    NotEqual,
    Equal,
}

/// Parser for comparison operators (==, !=, <, >, <=, >=)
pub fn comparison_op<'t, I>() -> impl Parser<'t, I, BinOp, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use BinOp::*;
    use Token::*;

    select! {
        Eq => Equal,
        NotEq => NotEqual,
        Less => LessThan,
        Greater => GreaterThan,
        LessEq => LessThanOrEqual,
        GreaterEq => GreaterThanOrEqual,
    }
}

/// Parser for arithmetic operators (+, -, *, /)
pub fn arithmetic_op<'t, I>() -> impl Parser<'t, I, BinOp, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use BinOp::*;
    use Token::*;

    select! {
        Plus => Add,
        Minus => Subtract,
        Star => Multiply,
        Slash => Divide,
    }
}
