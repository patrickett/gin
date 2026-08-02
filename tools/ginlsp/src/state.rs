use analysis::PackageCache;
use crossbeam_channel::unbounded;
use diagnostic::DiagnosticPathExt;
use resolve::GinPackageExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct JsonDocumentState {
    pub source: String,
}

pub struct GinSnapshot {
    pub cache: Arc<PackageCache>,
}

impl Clone for GinSnapshot {
    fn clone(&self) -> Self {
        Self {
            cache: Arc::clone(&self.cache),
        }
    }
}

/// Information about the files belonging to a Gin package.
pub struct PackageInfo {
    /// All `.gin` file paths discovered in the package source directory.
    pub file_paths: Vec<PathBuf>,
}

pub struct GinHost {
    pub cache: Arc<PackageCache>,
}

impl GinHost {
    pub fn new() -> Self {
        let (tx, _rx) = unbounded();
        Self {
            cache: Arc::new(PackageCache::new(tx)),
        }
    }

    pub fn snapshot(&self) -> GinSnapshot {
        GinSnapshot {
            cache: Arc::clone(&self.cache),
        }
    }

    /// Evict the cached package entry for the package containing `path`.
    /// Called when a file tab is closed to free typed AST memory.
    pub fn evict_package_for(&self, path: &Path) {
        let path = path.normalize_diagnostic_path();
        self.cache.invalidate_package_cache_for(&path);
    }

    /// Upsert a file into the database.
    pub fn upsert_file(&mut self, path: PathBuf, contents: String) {
        let path = path.normalize_diagnostic_path();
        if !self.cache.contains(&path) {
            let _ = self.cache.add_file(path.clone());
        }
        self.cache.set_contents(&path, contents);
    }

    /// Discover all `.gin` files under `dir`, load them into the database,
    /// and return a [`PackageInfo`] describing the package.
    ///
    /// This mirrors the file-collection logic that `ginc` uses so that
    /// the LSP sees the same set of files as the CLI.
    pub fn load_package(&mut self, dir: &Path) -> PackageInfo {
        let file_paths: Vec<PathBuf> = dir
            .collect_gin_files()
            .into_iter()
            .map(|path| path.normalize_diagnostic_path())
            .collect();
        for p in &file_paths {
            if !self.cache.contains(p) {
                let _ = self.cache.add_file(p.clone());
            }
        }
        PackageInfo { file_paths }
    }
}
