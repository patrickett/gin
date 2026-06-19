use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ReferencesInFile;
use ast::source::{LineIndex, SourceExt};
use parser::query::ParseOutput;
use typecheck::CompletionCandidate;
use typecheck::completions::FileAstCompletionExt;

/// Alias for `Arc<String>` used in source-and-parse results.
/// Cloning is O(1) instead of O(n) for the raw string.
pub type SharedSource = Arc<String>;

/// Cross-file typed transform result for one file in a package.
#[derive(Clone, PartialEq)]
pub struct FileTypedOutput {
    pub typed: typecheck::TypedFileAst,
}

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
    /// Pre-computed line starts for fast LSP position conversion.
    pub line_index: Arc<LineIndex>,
}

/// Gin-domain query engine — abstracts the analysis cache from LSP consumers.
///
/// Every method uses domain types (paths, diagnostics, type environments)
/// instead of internal cache plumbing.
///
/// The canonical implementation is [`CacheEngine`](crate::CacheEngine).
pub trait QueryEngine: Send {
    /// Register a file path and load its contents from disk.
    fn add_file(&mut self, path: PathBuf) -> Result<(), String>;

    /// Update a file's contents for an already-registered file.
    fn set_contents(&mut self, path: &Path, contents: String);

    /// Get the source text and parse output for a file.
    /// The source is returned as [`SharedSource`] (an `Arc<String>`) so that
    /// callers can clone it without copying the string contents.
    fn source_and_parse(&self, path: &Path) -> Option<(SharedSource, Arc<ParseOutput>)>;

    /// Get cached parse output for a file only.
    fn parse_output(&self, path: &Path) -> Option<Arc<ParseOutput>>;

    /// Type flaws and non-fatal parse symptoms for a package.
    fn package_semantics(&self, paths: &[PathBuf]) -> Vec<FileSemanticOutput>;

    /// All package diagnostics (type flaws, prepare symptoms, import symptoms) in one call.
    fn all_diagnostics(
        &self,
        paths: &[PathBuf],
        dependency_dirs: &std::collections::HashMap<String, PathBuf>,
    ) -> std::collections::HashMap<PathBuf, Vec<diagnostic::Diagnostic>>;

    /// Merged tag types across all files in the package containing `path`.
    fn tag_types_for_file(
        &self,
        path: &Path,
    ) -> Option<std::collections::HashMap<internment::Intern<String>, ast::ty::Ty>>;

    /// Hover at a byte position (parse-only, import, then typed; presentation applied).
    fn hover(&self, path: &Path, byte_pos: u32) -> Option<HoverContent>;

    /// Goto-definition target at a byte position (parse + resolve navigation).
    fn goto_definition(&self, path: &Path, byte_pos: u32) -> Option<resolve::CursorDefinition>;

    /// Completion candidates at a byte position (package tag types when available).
    fn completions_at(&self, path: &Path, byte_pos: u32) -> Vec<CompletionCandidate> {
        let Some((source, output)) = self.source_and_parse(path) else {
            return Vec::new();
        };
        let byte_pos = byte_pos as usize;
        let tag_types = self.tag_types_for_file(path);
        output
            .ast
            .completions_for_context(&source, byte_pos, tag_types.as_ref())
    }

    /// References to the symbol at `byte_pos` in the current file.
    fn references_at(&self, path: &Path, byte_pos: u32) -> Option<ReferencesInFile> {
        let (source, output) = self.source_and_parse(path)?;
        let byte_pos = byte_pos as usize;
        let word = output
            .ast
            .word_at_byte(byte_pos, &source)
            .or_else(|| source.word_at_byte_offset(byte_pos))?;
        let spans = output.ast.find_references(&word);
        if spans.is_empty() {
            return None;
        }
        Some(ReferencesInFile { spans })
    }

    /// All registered file paths.
    fn file_paths(&self) -> Vec<PathBuf>;

    /// Check if a file is registered.
    fn contains(&self, path: &Path) -> bool;

    /// Drop the cached package-level typed AST entry for the package
    /// containing `path`. Default implementation is a no-op.
    fn invalidate_package_cache_for(&self, _path: &Path) {}

    /// Clone the engine for concurrent snapshot access.
    fn snapshot(&self) -> Box<dyn QueryEngine>;

    /// Get a consistent document snapshot containing source text, parse
    /// output, and a line index for fast LSP position conversion.
    ///
    /// The default implementation builds a fresh `LineIndex` from the source
    /// returned by `source_and_parse`. Subtypes (e.g. `CacheEngine`) SHOULD
    /// override this with a cached line index.
    fn document_snapshot(&self, path: &Path) -> Option<DocumentSnapshot> {
        let path = path.to_path_buf();
        let (source, parse) = self.source_and_parse(&path)?;
        let line_index = Arc::new(LineIndex::new(&source));
        Some(DocumentSnapshot {
            path,
            source,
            parse,
            line_index,
        })
    }
}
