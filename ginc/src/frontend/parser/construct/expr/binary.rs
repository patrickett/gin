use crate::frontend::prelude::*;
use chumsky::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binary {
    pub lhs: Box<Expr>,
    pub op: BinOp,
    pub rhs: Box<Expr>,
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
    P: Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
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

/// Parser for any binary operator (comparison or arithmetic)
pub fn bin_op<'t, I>() -> impl Parser<'t, I, BinOp, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    comparison_op().or(arithmetic_op())
}

pub fn binary_expr<'t, I, P>(expr: P) -> impl Parser<'t, I, Binary, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
    P: Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
{
    expr.clone()
        .then(bin_op().then(expr))
        .map(|(lhs, (op, rhs))| Binary {
            lhs: Box::new(lhs),
            op,
            rhs: Box::new(rhs),
        })
}
