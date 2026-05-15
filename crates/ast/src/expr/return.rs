use crate::expr::Expr;
use crate::span::{SpanId, Spanned};

// TODO: make this Spanned<Return>
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Return {
    pub value: Option<Box<Spanned<Expr>>>,
    pub span_id: SpanId,
}
