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
    /// +
    Add,
    /// /
    Divide,
    /// *
    Multiply,
    /// -
    Subtract,
    /// :=
    Assign,
}
