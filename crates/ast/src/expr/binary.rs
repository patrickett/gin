use crate::expr::Expr;
use crate::span::Spanned;

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
