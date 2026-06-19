use crate::span::SpanId;
use internment::Intern;

/// A multi‑segment identifier.
///
/// `root` and every element of `segments` are **owned** strings.
/// This removes the need for a `'src` lifetime and allows the type to be
/// stored in caches or sent across threads safely.
///
/// The span for the overall `ModPath` is stored in its `Spanned<ModPath>` wrapper.
/// Per-segment spans (`root_span`, `segment_spans`) allow precise cursor targeting
/// within qualified paths (e.g. which segment of `core.maybe.Some` is under the cursor).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModPath {
    /// The root of a path can be a name mapped in `flask.jsonc` or
    /// a child folder in the current directory.
    ///
    /// NOTE: If there is a name conflict it will error.
    pub root: Intern<String>,
    pub segments: Vec<Intern<String>>,
    /// Span of the root segment. May be [`SpanId::INVALID`] when unknown
    /// (e.g. constructed during rewriting or in tests).
    pub root_span: SpanId,
    /// Spans of each segment, in the same order as `segments`.
    /// May be empty (when no segments) or contain [`SpanId::INVALID`] entries.
    pub segment_spans: Vec<SpanId>,
}

impl ModPath {
    /// Construct a new `ModPath` from an owned root and segment list.
    ///
    /// Per-segment spans are set to [`SpanId::INVALID`].
    /// Prefer [`ModPath::new_with_spans`] when spans are available.
    pub fn new(root: Intern<String>, segments: Vec<Intern<String>>) -> Self {
        Self {
            root,
            segments,
            root_span: SpanId::INVALID,
            segment_spans: Vec::new(),
        }
    }

    /// Construct a new `ModPath` with per-segment span information.
    pub fn new_with_spans(
        root: Intern<String>,
        root_span: SpanId,
        segments: Vec<Intern<String>>,
        segment_spans: Vec<SpanId>,
    ) -> Self {
        Self {
            root,
            segments,
            root_span,
            segment_spans,
        }
    }

    /// Return the number of segments (excluding root).
    pub fn num_segments(&self) -> usize {
        self.segments.len()
    }

    /// Return true if there are no segments (bare root-only path).
    pub fn is_bare(&self) -> bool {
        self.segments.is_empty()
    }

    /// Build the qualified string for this path, e.g. `"core.maybe.Some"`.
    pub fn to_qualified_string(&self) -> String {
        let mut s = self.root.to_string();
        for seg in &self.segments {
            s.push('.');
            s.push_str(seg.as_str());
        }
        s
    }
}
