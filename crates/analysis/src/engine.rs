use std::path::PathBuf;
use std::sync::Arc;

use ast::source::LineIndex;
use parser::query::ParseOutput;

/// Alias for `Arc<String>` used in source-and-parse results.
/// Cloning is O(1) instead of O(n) for the raw string.
pub type SharedSource = Arc<String>;

/// Per-file semantic output: type flaws + non-fatal parse symptoms.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileSemanticOutput {
    pub symptoms: Vec<diagnostic::Diagnostic>,
}

/// Semantic hover result: markdown body and optional source byte range for the LSP highlight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverContent {
    pub markdown: String,
    pub byte_range: Option<std::ops::Range<usize>>,
}

/// A consistent snapshot of a single file's parsed state: source text,
/// parse output, and a pre-computed line index for fast LSP position
/// conversion.
///
/// All three fields originate from the same `ParseEntry` and are returned
/// together so callers never observe inconsistent state even when the
/// underlying cache is mutated concurrently.
#[derive(Clone)]
pub struct DocumentSnapshot {
    /// Absolute path of the file.
    pub path: PathBuf,
    /// Source text (`Arc<String>` — cheap to clone).
    pub source: SharedSource,
    /// Parsed AST and diagnostics.
    pub parse: Arc<ParseOutput>,
    /// Pre-computed line-start offsets for fast LSP position conversion.
    pub line_index: Arc<LineIndex>,
}
