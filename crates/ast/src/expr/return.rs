use crate::expr::Expr;
use crate::expr::Typed;
use crate::span::SpanId;

// TODO: make this Spanned<Return>
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Return {
    pub value: Option<Box<Typed<Expr>>>,
    pub span_id: SpanId,
}
