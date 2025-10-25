use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub struct Binary<'src> {
    lhs: Box<Expr<'src>>,
    op: BinOp,
    rhs: Box<Expr<'src>>,
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
