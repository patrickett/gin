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

    /// Bits of an id that index spans inside one file's table.
    pub const SPAN_BITS_PER_FILE: u32 = 22;
    /// Spans one file table may hold before its ids would name the next file.
    pub const SPANS_PER_FILE: u32 = 1 << Self::SPAN_BITS_PER_FILE;
    /// File indices below this keep every file id clear of [`SpanId::INVALID`].
    pub const FILES_MAX: u32 = (1 << (32 - Self::SPAN_BITS_PER_FILE)) - 1;

    /// The file whose table issued this id; meaningful only for ids from
    /// [`SpanTable::for_file`] tables, which are unique across a module's files.
    #[inline]
    #[must_use]
    pub const fn file(self) -> u32 {
        self.0 >> Self::SPAN_BITS_PER_FILE
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
/// the keyword, or just the `while` keyword inside a while-loop).
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpanTable {
    spans: Vec<Span>,
    /// Id of the first span; ids below it belong to another file's table.
    base: u32,
    /// First id this table may not issue.
    limit: u32,
    /// Set once an insert found the table full; see [`SpanTable::insert`].
    overflowed: bool,
}

impl Default for SpanTable {
    fn default() -> Self {
        Self {
            spans: Vec::new(),
            base: 0,
            limit: u32::MAX,
            overflowed: false,
        }
    }
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
            ..Self::default()
        }
    }

    /// Create the table for file `file` of a multi-file module. Its ids are unique
    /// across the module and name their file, so one checker resolves any of them.
    pub fn for_file(file: u32) -> Self {
        assert!(file < SpanId::FILES_MAX);
        let base = file << SpanId::SPAN_BITS_PER_FILE;
        Self {
            spans: Vec::new(),
            base,
            limit: base + SpanId::SPANS_PER_FILE,
            overflowed: false,
        }
    }

    /// Insert a new span and return its SpanId.
    ///
    /// A full table never issues an id of another file's range. It records the
    /// overflow and returns its last id, so callers keep a valid, in-file id while
    /// the producer checks [`SpanTable::overflowed`] and discards the result.
    pub fn insert(&mut self, span: Span) -> SpanId {
        let id = self.base + self.spans.len() as u32;
        if id >= self.limit {
            assert!(!self.spans.is_empty());
            self.overflowed = true;
            return SpanId(self.limit - 1);
        }
        self.spans.push(span);
        SpanId(id)
    }

    /// Whether any insert found the table full; ids it returned then are unreliable.
    #[inline]
    #[must_use]
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Insert a span from a byte range and return its SpanId.
    pub fn insert_range(&mut self, range: std::ops::Range<usize>) -> SpanId {
        self.insert(Span::from_range(range))
    }

    /// Get the span data for a given SpanId.
    /// Returns a zero-length span at position 0 for invalid IDs.
    pub fn get(&self, id: SpanId) -> Span {
        let index = id.into_inner().checked_sub(self.base);
        match index {
            Some(index) if id.is_valid() && (index as usize) < self.spans.len() => {
                self.spans[index as usize]
            }
            _ => Span { start: 0, end: 0 },
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

    /// Get a reference to the inner value.
    #[inline]
    pub fn value(&self) -> &T {
        &self.value
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
