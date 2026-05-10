use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Gin-domain query engine — the deep seam that hides Salsa from callers.
///
/// Every method uses domain types (paths, diagnostics, type environments)
/// instead of Salsa internals (`File`, `Database`, `Accumulator`).
///
/// Two adapters exist:
/// - [`SalsaQueryEngine`](crate::SalsaQueryEngine): incremental, file-watching, production
/// - `SimpleQueryEngine`: re-parses every time (tests, single-shot compilation)
pub trait QueryEngine: Send {
    /// Register a file path and load its contents from disk.
    fn add_file(&mut self, path: PathBuf) -> Result<(), String>;

    /// Update a file's contents for an already-registered file.
    fn set_contents(&mut self, path: &Path, contents: String);

    /// Get the source text and parse output for a file.
    fn source_and_parse(&self, path: &Path) -> Option<(String, Arc<parser::ParseOutput>)>;

    /// Get cached parse output for a file only.
    fn parse_output(&self, path: &Path) -> Option<Arc<parser::ParseOutput>>;

    /// Type-check a set of files sharing one type environment.
    /// Returns per-path diagnostics in the same order as `paths`.
    fn typecheck_package(
        &self,
        paths: &[PathBuf],
    ) -> Vec<Vec<diagnostic::Diagnostic>>;

    /// Build a shared type environment for a set of files.
    fn package_ty_env(&self, paths: &[PathBuf]) -> Arc<typeck::TyEnv>;

    /// Compute hover markdown at a byte position in a file.
    fn hover(&self, path: &Path, byte_pos: u32) -> Option<String>;

    /// All registered file paths.
    fn file_paths(&self) -> Vec<PathBuf>;

    /// Check if a file is registered.
    fn contains(&self, path: &Path) -> bool;

    /// Clone the engine for concurrent snapshot access.
    fn snapshot(&self) -> Box<dyn QueryEngine>;
}
