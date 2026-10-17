use crate::{Pattern, Spanned};

use super::{Expr, Typed};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Condition {
    Proposition {
        subject: Box<Typed<Expr>>,
    },
    Compare {
        left: Box<Typed<Expr>>,
        relation: crate::ProofRelation,
        right: Box<Typed<Expr>>,
    },
    Is {
        subject: Box<Typed<Expr>>,
        pattern: Box<Spanned<Pattern>>,
    },
    Not(Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
}
