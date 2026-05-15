//! Span types re-exported from the `span` crate.
//!
//! The canonical definitions of `Span`, `SpanId`, `SpanTable`, and `Spanned`
//! live in the `span` crate so that foundational crates (lexer, ast, diagnostic)
//! can share them without pulling in heavy transitive dependencies.

pub use span::{Span, SpanId, SpanTable, Spanned};

/// Lightweight trait for AST nodes that carry a single `SpanId`.
///
/// This is intentionally separate from `Spanned<T>` (which pairs an arbitrary `T` with a span).
pub trait HasSpanId {
    fn span_id(&self) -> SpanId;
}

// HasSpanId implementations for types that carry a pub span: SpanId field.
macro_rules! impl_has_span_id {
    ($ty:ty) => {
        impl HasSpanId for $ty {
            fn span_id(&self) -> SpanId {
                self.span
            }
        }
    };
}

impl_has_span_id!(crate::WhileLoop);
impl_has_span_id!(crate::ForInLoop);
impl_has_span_id!(crate::Binary);
impl_has_span_id!(crate::Range);
impl_has_span_id!(crate::WhenExpr);
impl_has_span_id!(crate::IfExpr);
impl_has_span_id!(crate::FormatString);

impl HasSpanId for crate::Return {
    fn span_id(&self) -> SpanId {
        self.span_id
    }
}
