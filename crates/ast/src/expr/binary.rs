use crate::prelude::*;
use chumsky::prelude::*;
use lexer::Token;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binary {
    pub lhs: Box<Spanned<Expr>>,
    pub op: BinOp,
    pub rhs: Box<Spanned<Expr>>,
}

impl Binary {
    pub fn new(lhs: Spanned<Expr>, op: BinOp, rhs: Spanned<Expr>) -> Self {
        Self {
            lhs: Box::new(lhs),
            op,
            rhs: Box::new(rhs),
        }
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
    Modulo,
    NotEqual,
    Equal,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}

impl BinOp {
    pub fn is_comparison(&self) -> bool {
        matches!(
            self,
            BinOp::Equal
                | BinOp::NotEqual
                | BinOp::LessThan
                | BinOp::LessThanOrEqual
                | BinOp::GreaterThan
                | BinOp::GreaterThanOrEqual
        )
    }
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

/// Parser for arithmetic operators (+, -, *, /, %)
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
        Percent => Modulo,
    }
}

/// Parser for bitwise operators (&, |, ^, <<, >>)
pub fn bitwise_op<'t, I>() -> impl Parser<'t, I, BinOp, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! {
        Token::Ampersand  => BinOp::BitAnd,
        Token::Pipe       => BinOp::BitOr,
        Token::Caret      => BinOp::BitXor,
        Token::ShiftLeft  => BinOp::ShiftLeft,
        Token::ShiftRight => BinOp::ShiftRight,
    }
}

