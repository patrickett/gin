use analysis::PackageCache;
use crossbeam_channel::unbounded;
use diagnostic::DiagnosticPathExt;
use resolve::{self, GinPackageExt};
use std::collections::HashMap;
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
    pub package_file_cache: HashMap<PathBuf, Vec<PathBuf>>,
}

impl GinHost {
    pub fn new() -> Self {
        let (tx, _rx) = unbounded();
        Self {
            cache: Arc::new(PackageCache::new(tx)),
            package_file_cache: HashMap::new(),
        }
    }

    pub fn snapshot(&self) -> GinSnapshot {
        GinSnapshot {
            cache: Arc::clone(&self.cache),
        }
    }

    /// Evict the cached package entry for the package containing `path`.
    /// Called when a file tab is closed to free typed AST memory.
    pub fn evict_package_for(&mut self, path: &Path) {
        let path = path.normalize_diagnostic_path();
        self.cache.invalidate_package_cache_for(&path);
        if let Some(root) = resolve::find_package_root(&path) {
            self.package_file_cache
                .remove(&root.normalize_diagnostic_path());
        }
    }

    pub fn evict_file_for(&mut self, path: &Path) {
        let path = path.normalize_diagnostic_path();
        self.evict_package_for(&path);
        self.cache.evict_file(&path);
        if let Some(root) = resolve::find_package_root(&path) {
            self.package_file_cache
                .remove(&root.normalize_diagnostic_path());
        }
    }

    /// Upsert a file into the database.
    pub fn upsert_file(&mut self, path: PathBuf, contents: String) {
        let path = path.normalize_diagnostic_path();
        let was_new_file = !self.cache.contains(&path);
        if !self.cache.contains(&path) {
            let _ = self.cache.add_file(path.clone());
        }
        self.cache.set_contents(&path, contents);
        if was_new_file && let Some(root) = resolve::find_package_root(&path) {
            self.package_file_cache
                .remove(&root.normalize_diagnostic_path());
        }
    }

    /// Discover all `.gin` files under `dir`, load them into the database,
    /// and return a [`PackageInfo`] describing the package.
    ///
    /// This mirrors the file-collection logic that `ginc` uses so that
    /// the LSP sees the same set of files as the CLI.
    pub fn load_package(&mut self, dir: &Path) -> PackageInfo {
        let dir = dir.normalize_diagnostic_path();
        if let Some(cached) = self.package_file_cache.get(&dir) {
            return PackageInfo {
                file_paths: cached.clone(),
            };
        }
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
        self.package_file_cache.insert(dir, file_paths.clone());
        PackageInfo { file_paths }
    }
}
