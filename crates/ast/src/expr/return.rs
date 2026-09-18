use crate::expr::Expr;
use crate::expr::Typed;
use crate::span::SpanId;

// TODO: make this Spanned<Return>
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Return {
    /// Values returned by this statement, in declaration order.
    pub values: Vec<Typed<Expr>>,
    pub span_id: SpanId,
}
