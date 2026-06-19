use crate::expr::Expr;
use crate::expr::Typed;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binary {
    pub lhs: Box<Typed<Expr>>,
    pub op: BinOp,
    pub rhs: Box<Typed<Expr>>,
}

impl Binary {
    pub fn new(lhs: Typed<Expr>, op: BinOp, rhs: Typed<Expr>) -> Self {
        Self {
            lhs: Box::new(lhs),
            op,
            rhs: Box::new(rhs),
        }
    }
}

/// Binary operations are defined as `lhs op rhs`
///
/// Comparison operators (`<`, `<=`, `>`, `>=`) are NOT represented
/// as `BinOp` variants. They are desugared to function calls during parsing
/// (see `parser::expr::try_comparison_op`), so they go through normal trait
/// method resolution like `lt(lhs, rhs)`, `gt(lhs, rhs)`, etc.
///
/// Equality (`is`) is handled natively by the compiler for all ADTs —
/// no desugaring or trait needed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Divide,
    Multiply,
    Subtract,
    Modulo,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
    LogicalAnd,
    LogicalOr,
}

impl BinOp {
    pub fn symbol(&self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Divide => "/",
            BinOp::Multiply => "*",
            BinOp::Subtract => "-",
            BinOp::Modulo => "%",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::ShiftLeft => "<<",
            BinOp::ShiftRight => ">>",
            BinOp::LogicalAnd => "and",
            BinOp::LogicalOr => "or",
        }
    }
}
