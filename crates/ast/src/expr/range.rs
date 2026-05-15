use crate::expr::Expr;
use crate::span::{SpanId, Spanned};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: Box<Spanned<Expr>>,
    pub end: Box<Spanned<Expr>>,
    pub span: SpanId,
}

impl Range {
    pub fn new(start: Spanned<Expr>, end: Spanned<Expr>, span: SpanId) -> Self {
        Self {
            start: Box::new(start),
            end: Box::new(end),
            span,
        }
    }
}
