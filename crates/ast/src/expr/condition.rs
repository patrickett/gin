use crate::{Pattern, Spanned};

use super::{Expr, Typed};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Condition {
    Is {
        subject: Box<Typed<Expr>>,
        pattern: Box<Spanned<Pattern>>,
    },
    Not(Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
}
