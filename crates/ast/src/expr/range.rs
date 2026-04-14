use crate::expr::Expr;
use crate::span::Spanned;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: Box<Spanned<Expr>>,
    pub end: Box<Spanned<Expr>>,
}

impl Range {
    pub fn new(start: Spanned<Expr>, end: Spanned<Expr>) -> Self {
        Self {
            start: Box::new(start),
            end: Box::new(end),
        }
    }
}
