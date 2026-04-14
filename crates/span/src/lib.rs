//! Span handling with ID-based optimization.
//!
//! This crate provides a memory-efficient span representation using [`SpanId`]
//! instead of storing full span data in every AST node and token.
//!
//! These types have zero external dependencies so that foundational crates
//! (lexer, ast, diagnostic) can share them without pulling in heavy transitive
//! dependencies.

use std::ops::{Deref, DerefMut};

/// A unique identifier for a span in the span table.
///
/// Using u32 instead of usize saves 4 bytes per span reference on 64-bit systems.
/// This is a significant memory savings when spans are stored in thousands of
/// AST nodes and tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpanId(pub(crate) u32);

impl SpanId {
    /// Sentinel value representing an invalid/unknown span.
    pub const INVALID: Self = Self(u32::MAX);

    /// Create a new SpanId from a raw u32 value.
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Get the raw u32 value.
    pub const fn into_inner(self) -> u32 {
        self.0
    }

    /// Check if this SpanId is valid (not INVALID).
    pub const fn is_valid(self) -> bool {
        self.0 != u32::MAX
    }
}

impl Default for SpanId {
    fn default() -> Self {
        Self::INVALID
    }
}

/// The actual span data - byte range in source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// Byte offset of the start of the span.
    pub start: usize,
    /// Byte offset of the end of the span (exclusive).
    pub end: usize,
}

impl Span {
    /// Create a new span with start and end positions.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Create a span from a byte range.
    pub fn from_range(range: std::ops::Range<usize>) -> Self {
        Self {
            start: range.start,
            end: range.end,
        }
    }

    /// Get the length of the span in bytes.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Check if the span is empty.
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Merge two spans into a larger span that covers both.
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Check if a byte position falls within this span.
    pub fn contains(&self, byte_pos: usize) -> bool {
        self.start <= byte_pos && byte_pos <= self.end
    }

    /// Convert this span to a Range for use with string slicing.
    pub fn as_range(&self) -> std::ops::Range<usize> {
        self.start..self.end
    }

    /// Extract the substring from source text that this span covers.
    pub fn extract(self, source: &str) -> &str {
        &source[self.as_range()]
    }
}

/// A table that stores all spans and maps SpanIds to Span data.
///
/// This enables interned span storage - each unique span is stored once and
/// referenced by ID throughout the AST. This dramatically reduces memory usage
/// compared to storing full span data in every node.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SpanTable {
    spans: Vec<Span>,
}

impl SpanTable {
    /// Create a new empty span table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new span and return its SpanId.
    pub fn insert(&mut self, span: Span) -> SpanId {
        let id = self.spans.len() as u32;
        self.spans.push(span);
        SpanId(id)
    }

    /// Insert a span from a byte range and return its SpanId.
    pub fn insert_range(&mut self, range: std::ops::Range<usize>) -> SpanId {
        self.insert(Span::from_range(range))
    }

    /// Get the span data for a given SpanId.
    /// Returns a zero-length span at position 0 for invalid IDs.
    pub fn get(&self, id: SpanId) -> Span {
        if id.is_valid() && (id.into_inner() as usize) < self.spans.len() {
            self.spans[id.into_inner() as usize]
        } else {
            Span { start: 0, end: 0 }
        }
    }

    /// Get the span data for a given SpanId, returning None if invalid.
    pub fn try_get(&self, id: SpanId) -> Option<Span> {
        if id.is_valid() {
            self.spans.get(id.into_inner() as usize).copied()
        } else {
            None
        }
    }

    /// Create a new span that merges two existing spans by their IDs.
    /// The merged span is automatically added to the table.
    pub fn merge(&mut self, a: SpanId, b: SpanId) -> SpanId {
        let span_a = self.get(a);
        let span_b = self.get(b);
        self.insert(span_a.merge(span_b))
    }

    /// Check if a byte position is within a span by its ID.
    pub fn contains(&self, id: SpanId, byte_pos: usize) -> bool {
        self.get(id).contains(byte_pos)
    }

    /// Get the total number of spans in the table.
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// Check if the table is empty.
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Reserve capacity for additional spans to reduce allocations.
    pub fn reserve(&mut self, additional: usize) {
        self.spans.reserve(additional);
    }

    /// Clear all spans from the table.
    pub fn clear(&mut self) {
        self.spans.clear()
    }
}

/// A value paired with its source span identifier.
///
/// This is the primary way spans are attached to values in the AST.
/// Instead of storing full span data (start, end) in every node, we store
/// just a SpanId that references the span data in a SpanTable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Spanned<T>(pub T, pub SpanId);

impl<T> Spanned<T> {
    /// Create a new Spanned value.
    pub fn new(value: T, span_id: SpanId) -> Self {
        Self(value, span_id)
    }

    /// Split into the value and span ID.
    pub fn into_parts(self) -> (T, SpanId) {
        (self.0, self.1)
    }

    /// Get a reference to the inner value.
    pub fn value(&self) -> &T {
        &self.0
    }

    /// Get the inner value by value.
    pub fn into_value(self) -> T {
        self.0
    }

    /// Get the span ID.
    pub fn span_id(&self) -> SpanId {
        self.1
    }

    /// Map the inner value while preserving the span ID.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned(f(self.0), self.1)
    }

    /// Map the inner value with a fallible function while preserving the span ID.
    pub fn try_map<U, E>(self, f: impl FnOnce(T) -> Result<U, E>) -> Result<Spanned<U>, E> {
        Ok(Spanned(f(self.0)?, self.1))
    }

    /// Create a Spanned value by inserting a span into the table.
    /// This is useful when you have access to a SpanTable.
    pub fn with_span(table: &mut SpanTable, value: T, span: Span) -> Self {
        Self(value, table.insert(span))
    }

    /// Create a Spanned value from a byte range.
    pub fn with_range(table: &mut SpanTable, value: T, range: std::ops::Range<usize>) -> Self {
        Self(value, table.insert_range(range))
    }

    /// Get the actual span data from a span table.
    pub fn resolve_span(&self, table: &SpanTable) -> Span {
        table.get(self.1)
    }

    /// Extract the source text this span covers.
    pub fn extract_source<'src>(&self, table: &SpanTable, source: &'src str) -> &'src str {
        self.resolve_span(table).extract(source)
    }
}

impl<T> Deref for Spanned<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Spanned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_id_operations() {
        let id = SpanId::new(42);
        assert!(id.is_valid());
        assert_eq!(id.into_inner(), 42);
        assert!(!SpanId::INVALID.is_valid());
        assert_eq!(SpanId::default(), SpanId::INVALID);
    }

    #[test]
    fn span_operations() {
        let span = Span::new(10, 20);
        assert_eq!(span.len(), 10);
        assert!(!span.is_empty());
        assert!(span.contains(15));
        assert!(!span.contains(5));
        assert!(!span.contains(25));
        assert_eq!(span.as_range(), 10..20);
    }

    #[test]
    fn span_merge() {
        let a = Span::new(10, 20);
        let b = Span::new(15, 30);
        let merged = a.merge(b);
        assert_eq!(merged.start, 10);
        assert_eq!(merged.end, 30);
    }

    #[test]
    fn span_table() {
        let mut table = SpanTable::new();
        let id1 = table.insert_range(10..20);
        let id2 = table.insert_range(30..40);
        assert_eq!(table.len(), 2);
        assert_eq!(table.get(id1), Span::new(10, 20));
        assert_eq!(table.get(id2), Span::new(30, 40));
        assert!(table.contains(id1, 15));
        assert!(!table.contains(id1, 25));
    }

    #[test]
    fn spanned_operations() {
        let mut table = SpanTable::new();
        let spanned = Spanned::with_range(&mut table, "hello", 5..10);
        assert_eq!(spanned.value(), &"hello");
        assert!(spanned.span_id().is_valid());
        assert_eq!(spanned.resolve_span(&table), Span::new(5, 10));
        let span_id = spanned.span_id();
        let uppercased = spanned.map(|s| s.to_uppercase());
        assert_eq!(uppercased.value(), &"HELLO");
        assert_eq!(uppercased.span_id(), span_id);
    }

    #[test]
    fn extract_source() {
        let source = "fn main() { return 42; }";
        let mut table = SpanTable::new();
        let spanned = Spanned::with_range(
            &mut table,
            "return",
            source.find("return").unwrap()..source.find("return").unwrap() + 6,
        );
        assert_eq!(spanned.extract_source(&table, source), "return");
    }
}
