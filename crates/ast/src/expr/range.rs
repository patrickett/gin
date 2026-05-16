use crate::expr::Expr;
use crate::expr::Typed;
use crate::span::SpanId;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: Box<Typed<Expr>>,
    pub end: Box<Typed<Expr>>,
    pub span: SpanId,
}

impl Range {
    pub fn new(start: Typed<Expr>, end: Typed<Expr>, span: SpanId) -> Self {
        Self {
            start: Box::new(start),
            end: Box::new(end),
            span,
        }
    }
}
