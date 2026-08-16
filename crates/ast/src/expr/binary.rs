use crate::expr::Expr;
use crate::expr::Typed;
use strum::{EnumString, IntoStaticStr};

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
/// Pattern relations (`is`) have their own AST representation because their
/// right-hand side is a pattern rather than a value expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash, IntoStaticStr)]
pub enum BinOp {
    #[strum(serialize = "=")]
    Equal,
    #[strum(serialize = "<")]
    Less,
    #[strum(serialize = "<=")]
    LessOrEqual,
    #[strum(serialize = ">=")]
    GreaterOrEqual,
    #[strum(serialize = ">")]
    Greater,
    #[strum(serialize = "+")]
    Add,
    #[strum(serialize = "/")]
    Divide,
    #[strum(serialize = "*")]
    Multiply,
    #[strum(serialize = "-")]
    Subtract,
    #[strum(serialize = "%")]
    Modulo,
    #[strum(serialize = "&")]
    BitAnd,
    #[strum(serialize = "|")]
    BitOr,
    #[strum(serialize = "^")]
    BitXor,
    #[strum(serialize = "<<")]
    ShiftLeft,
    #[strum(serialize = ">>")]
    ShiftRight,
    #[strum(serialize = "and")]
    LogicalAnd,
    #[strum(serialize = "or")]
    LogicalOr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString)]
pub enum OperatorRole {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
}

impl OperatorRole {
    pub fn for_bin_op(op: &BinOp) -> Option<Self> {
        match op {
            BinOp::Add => Some(Self::Add),
            BinOp::Subtract => Some(Self::Subtract),
            BinOp::Multiply => Some(Self::Multiply),
            BinOp::Divide => Some(Self::Divide),
            BinOp::Modulo => Some(Self::Modulo),
            BinOp::BitAnd => Some(Self::BitAnd),
            BinOp::BitOr => Some(Self::BitOr),
            BinOp::BitXor => Some(Self::BitXor),
            BinOp::ShiftLeft => Some(Self::ShiftLeft),
            BinOp::ShiftRight => Some(Self::ShiftRight),
            BinOp::Less => Some(Self::Less),
            BinOp::LessOrEqual => Some(Self::LessOrEqual),
            BinOp::Greater => Some(Self::Greater),
            BinOp::GreaterOrEqual => Some(Self::GreaterOrEqual),
            BinOp::Equal => Some(Self::Equal),
            _ => None,
        }
    }
}
