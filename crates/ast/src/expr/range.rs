use crate::expr::Expr;
use crate::expr::Typed;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: Box<Typed<Expr>>,
    pub end: Box<Typed<Expr>>,
}

impl Range {
    pub fn new(start: Typed<Expr>, end: Typed<Expr>) -> Self {
        Self {
            start: Box::new(start),
            end: Box::new(end),
        }
    }
}
