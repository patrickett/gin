use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub struct Binary {
    lhs: Box<Expr>,
    op: BinOp,
    rhs: Box<Expr>,
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
