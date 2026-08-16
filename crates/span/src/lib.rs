//! Span handling with ID-based optimization.
//!
//! This crate provides a memory-efficient span representation using [`SpanId`]
//! instead of storing full span data in every AST node and token.
//!
//! These types have zero external dependencies so that foundational crates
//! (lexer, ast, diagnostic) can share them without pulling in heavy transitive
//! dependencies.

use derive_more::From;
use std::ops::{Deref, DerefMut};

#[cfg(test)]
extern crate self as span;

/// A unique identifier for a span in the span table.
///
/// Using u32 instead of usize saves 4 bytes per span reference on 64-bit systems.
/// This is a significant memory savings when spans are stored in thousands of
/// AST nodes and tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, From)]
pub struct SpanId(pub(crate) u32);

impl SpanId {
    /// Sentinel value representing an invalid/unknown span.
    pub const INVALID: Self = Self(u32::MAX);

    /// Create a new SpanId from a raw u32 value.
    #[inline]
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Get the raw u32 value.
    #[inline]
    #[must_use]
    pub const fn into_inner(self) -> u32 {
        self.0
    }

    /// Check if this SpanId is valid (not INVALID).
    #[inline]
    #[must_use]
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
///
/// Stored as `u32` offsets to halve memory footprint vs `usize` on 64-bit
/// platforms. Source files exceeding 4 GiB are not supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// Byte offset of the start of the span (internal: u32).
    /// Use [`Span::start`] to get a `usize` for source slicing.
    pub(crate) start: u32,
    /// Byte offset of the end of the span (internal: u32).
    /// Use [`Span::end`] to get a `usize` for source slicing.
    pub(crate) end: u32,
}

impl Span {
    /// Create a new span with start and end positions.
    #[inline]
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self {
            start: start as u32,
            end: end as u32,
        }
    }

    /// Create a span from a byte range.
    #[inline]
    #[must_use]
    pub fn from_range(range: std::ops::Range<usize>) -> Self {
        Self {
            start: range.start as u32,
            end: range.end as u32,
        }
    }

    /// Get the length of the span in bytes.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        (self.end as usize).saturating_sub(self.start as usize)
    }

    /// Check if the span is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Merge two spans into a larger span that covers both.
    #[inline]
    #[must_use]
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Check if a byte position falls within this span.
    #[inline]
    #[must_use]
    pub fn contains(&self, byte_pos: usize) -> bool {
        (self.start as usize) <= byte_pos && byte_pos < (self.end as usize)
    }

    /// Convert this span to a Range for use with string slicing.
    #[inline]
    #[must_use]
    pub fn to_range(&self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }

    /// Return the start offset as `usize` (for source slicing and arithmetic with `usize` values).
    #[inline]
    #[must_use]
    pub fn start(&self) -> usize {
        self.start as usize
    }

    /// Return the end offset as `usize` (for source slicing and arithmetic with `usize` values).
    #[inline]
    #[must_use]
    pub fn end(&self) -> usize {
        self.end as usize
    }

    /// Return the start offset as `u32` (for comparison with other `u32` span offsets).
    #[inline]
    #[must_use]
    pub fn start_u32(&self) -> u32 {
        self.start
    }

    /// Return the end offset as `u32` (for comparison with other `u32` span offsets).
    #[inline]
    #[must_use]
    pub fn end_u32(&self) -> u32 {
        self.end
    }

    /// Extract the substring from source text that this span covers.
    #[inline]
    pub fn extract(self, source: &str) -> &str {
        &source[self.to_range()]
    }
}

/// A span that represents a narrower range within some enclosing span.
///
/// Used inside expression variant structs that always live inside `Typed<T>`.
/// The full expression span is on the `Typed<T>` wrapper; `SubSpan` values
/// are more specific ranges within it (e.g. the body of an `if` excluding
/// the keyword, or just the `loop` keyword inside a while-loop).
///
/// This is a newtype rather than a bare `SpanId` to make the relationship
/// explicit at the type level: a `SubSpan` is *always* a subset of the
/// wrapping `Typed<T>`'s span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubSpan(pub(crate) SpanId);

impl SubSpan {
    /// Create a new SubSpan from a SpanId.
    #[inline]
    #[must_use]
    pub const fn new(id: SpanId) -> Self {
        Self(id)
    }

    /// Convert back to the raw SpanId.
    #[inline]
    #[must_use]
    pub const fn into_inner(self) -> SpanId {
        self.0
    }
}

/// A table that stores all spans and maps SpanIds to Span data.
///
/// This enables interned span storage - each unique span is stored once and
/// referenced by ID throughout the AST. This dramatically reduces memory usage
/// compared to storing full span data in every node.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct SpanTable {
    spans: Vec<Span>,
}

impl SpanTable {
    /// Create a new empty span table.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new span table with the given pre-allocated capacity.
    /// Use when you have a reasonable estimate of the number of spans
    /// to avoid repeated reallocation during lexing and parsing.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            spans: Vec::with_capacity(capacity),
        }
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
    #[inline]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// Check if the table is empty.
    #[inline]
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
pub struct Spanned<T> {
    pub value: T,
    pub span_id: SpanId,
}

impl<T> Spanned<T> {
    /// Create a new Spanned value.
    #[inline]
    pub fn new(value: T, span_id: SpanId) -> Self {
        Self { span_id, value }
    }

    /// Split into the value and span ID.
    #[inline]
    pub fn into_parts(self) -> (T, SpanId) {
        (self.value, self.span_id)
    }

    /// Get a reference to the inner value.
    #[inline]
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Get the inner value by value.
    #[inline]
    pub fn into_value(self) -> T {
        self.value
    }

    /// Get the span ID.
    #[inline]
    pub fn span_id(&self) -> SpanId {
        self.span_id
    }

    /// Map the inner value while preserving the span ID.
    #[inline]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            value: f(self.value),
            span_id: self.span_id,
        }
    }

    /// Map the inner value with a fallible function while preserving the span ID.
    #[inline]
    pub fn try_map<U, E>(self, f: impl FnOnce(T) -> Result<U, E>) -> Result<Spanned<U>, E> {
        Ok(Spanned {
            value: f(self.value)?,
            span_id: self.span_id,
        })
    }

    /// Create a Spanned value by inserting a span into the table.
    #[inline]
    pub fn with_span(table: &mut SpanTable, value: T, span: Span) -> Self {
        Self {
            value,
            span_id: table.insert(span),
        }
    }

    /// Create a Spanned value from a byte range.
    #[inline]
    pub fn with_range(table: &mut SpanTable, value: T, range: std::ops::Range<usize>) -> Self {
        Self {
            value,
            span_id: table.insert_range(range),
        }
    }

    /// Get the actual span data from a span table.
    #[inline]
    pub fn resolve_span(&self, table: &SpanTable) -> Span {
        table.get(self.span_id)
    }

    /// Extract the source text this span covers.
    #[inline]
    pub fn extract_source<'src>(&self, table: &SpanTable, source: &'src str) -> &'src str {
        self.resolve_span(table).extract(source)
    }
}

impl<T> Deref for Spanned<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for Spanned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}
#[cfg(test)]
#[path = "../tests/lib_tests.rs"]
mod tests;
