use crate::expr::Expr;
use crate::expr::Typed;
use crate::span::SpanId;

// TODO: make this Spanned<Return>
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Return {
    /// Values returned by this statement, in declaration order.
    pub values: Vec<Typed<Expr>>,
    pub postcondition: Option<Box<crate::Spanned<crate::ProofProposition>>>,
    pub span_id: SpanId,
}
