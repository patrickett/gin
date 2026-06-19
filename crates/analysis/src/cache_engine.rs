//! [`QueryEngine`] implementation backed by [`PackageCache`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crossbeam_channel::Sender;

use notify_debouncer_mini::DebounceEventResult;
use parser::query::ParseOutput;

use crate::engine::{
    DocumentSnapshot, FileSemanticOutput, HoverContent, QueryEngine, SharedSource,
};
use crate::package_cache::PackageCache;

/// [`QueryEngine`] adapter over [`PackageCache`].
///
/// # Thread safety
///
/// All operations use interior mutability (`Mutex` + `AtomicU64`). The engine
/// can be shared across threads via `Arc`. Callers that need mutable access
/// (e.g. LSP document sync) should synchronize externally if required by the
/// [`QueryEngine`] trait signature.
pub struct CacheEngine {
    cache: Arc<PackageCache>,
    /// File paths shared via `Arc` so that `snapshot()` does not clone the
    /// entire map. Only `add_file` (which takes `&mut self`) writes to this;
    /// all read-only operations (`file_paths`, `snapshot`) take a read lock.
    file_paths: Arc<RwLock<HashMap<PathBuf, ()>>>,
}

impl CacheEngine {
    pub fn new(tx: Sender<DebounceEventResult>) -> Self {
        Self {
            cache: Arc::new(PackageCache::new(tx)),
            file_paths: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl QueryEngine for CacheEngine {
    fn add_file(&mut self, path: PathBuf) -> Result<(), String> {
        self.cache.add_file(path.clone())?;
        self.file_paths.write().unwrap().insert(path, ());
        Ok(())
    }

    fn set_contents(&mut self, path: &Path, contents: String) {
        self.cache.set_contents(path, contents);
    }

    fn source_and_parse(&self, path: &Path) -> Option<(SharedSource, Arc<ParseOutput>)> {
        self.cache.source_and_parse(path)
    }

    fn parse_output(&self, path: &Path) -> Option<Arc<ParseOutput>> {
        self.cache.parse_output(path)
    }

    fn package_semantics(&self, paths: &[PathBuf]) -> Vec<FileSemanticOutput> {
        let outputs = match self.cache.package_semantics(paths) {
            Some(o) => o,
            None => return vec![FileSemanticOutput::default(); paths.len()],
        };
        self.remap_semantics(paths, &outputs)
    }

    fn tag_types_for_file(
        &self,
        path: &Path,
    ) -> Option<HashMap<internment::Intern<String>, ast::ty::Ty>> {
        self.cache.tag_types_for_file(path)
    }

    fn hover(&self, path: &Path, byte_pos: u32) -> Option<HoverContent> {
        self.cache.hover(path, byte_pos)
    }

    fn goto_definition(&self, path: &Path, byte_pos: u32) -> Option<resolve::CursorDefinition> {
        let (source, parse) = self.source_and_parse(path)?;
        resolve::cursor_definition(path, &parse.ast, &source, byte_pos as usize, &|p| {
            resolve::ParsedFile::read(p)
        })
    }

    fn all_diagnostics(
        &self,
        paths: &[PathBuf],
        dependency_dirs: &std::collections::HashMap<String, PathBuf>,
    ) -> std::collections::HashMap<PathBuf, Vec<diagnostic::Diagnostic>> {
        self.cache.all_diagnostics(paths, dependency_dirs)
    }

    fn file_paths(&self) -> Vec<PathBuf> {
        self.file_paths.read().unwrap().keys().cloned().collect()
    }

    fn contains(&self, path: &Path) -> bool {
        self.cache.contains(path)
    }

    fn invalidate_package_cache_for(&self, path: &Path) {
        self.cache.invalidate_package_cache_for(path);
    }

    fn snapshot(&self) -> Box<dyn QueryEngine> {
        // The cache uses interior mutability, so we can share the same cache
        // and file_paths across snapshots without cloning either.
        Box::new(Self {
            cache: Arc::clone(&self.cache),
            file_paths: Arc::clone(&self.file_paths),
        })
    }

    fn document_snapshot(&self, path: &Path) -> Option<DocumentSnapshot> {
        self.cache.document_snapshot(path)
    }
}

impl CacheEngine {
    /// Remap semantic outputs from sorted-by-path order to caller-requested order.
    fn remap_semantics(
        &self,
        requested: &[PathBuf],
        sorted: &[FileSemanticOutput],
    ) -> Vec<FileSemanticOutput> {
        if requested.is_empty() {
            return Vec::new();
        }
        let pkg_files = self
            .cache
            .get_or_compute_package(requested)
            .map(|e| e.file_paths.clone())
            .unwrap_or_default();

        let mut by_path: HashMap<&PathBuf, &FileSemanticOutput> = HashMap::new();
        for (fp, out) in pkg_files.iter().zip(sorted.iter()) {
            by_path.insert(fp, out);
        }
        requested
            .iter()
            .map(|p| by_path.get(p).copied().cloned().unwrap_or_default())
            .collect()
    }
}
