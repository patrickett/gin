//! Span types re-exported from the `span` crate.
//!
//! The canonical definitions of `Span`, `SpanId`, `SpanTable`, `Spanned`, and
//! `SubSpan` live in the `span` crate so that foundational crates (lexer, ast,
//! diagnostic) can share them without pulling in heavy transitive dependencies.

pub use span::{Span, SpanId, SpanTable, Spanned, SubSpan};

/// Lightweight trait for AST nodes that carry a single `SpanId`.
///
/// This is intentionally separate from `Spanned<T>` (which pairs an arbitrary `T` with a span).
pub trait HasSpanId {
    fn span_id(&self) -> SpanId;
}

impl HasSpanId for crate::WhileLoop {
    fn span_id(&self) -> SpanId {
        self.keyword_span.into_inner()
    }
}

impl HasSpanId for crate::ForInLoop {
    fn span_id(&self) -> SpanId {
        self.keyword_span.into_inner()
    }
}

impl HasSpanId for crate::WhenExpr {
    fn span_id(&self) -> SpanId {
        self.body_span.into_inner()
    }
}

impl HasSpanId for crate::IfExpr {
    fn span_id(&self) -> SpanId {
        self.body_span.into_inner()
    }
}

impl HasSpanId for crate::Return {
    fn span_id(&self) -> SpanId {
        self.span_id
    }
}
