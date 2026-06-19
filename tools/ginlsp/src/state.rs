use analysis::CacheEngine;
use analysis::QueryEngine;
use crossbeam_channel::unbounded;
use diagnostic::DiagnosticPathExt;
use resolve::GinPackageExt;
use std::path::{Path, PathBuf};

pub struct JsonDocumentState {
    pub source: String,
}

pub struct GinSnapshot {
    pub engine: Box<dyn QueryEngine>,
}

impl Clone for GinSnapshot {
    fn clone(&self) -> Self {
        Self {
            engine: self.engine.snapshot(),
        }
    }
}

/// Information about the files belonging to a Gin package.
pub struct PackageInfo {
    /// All `.gin` file paths discovered in the package source directory.
    pub file_paths: Vec<PathBuf>,
}

pub struct GinHost {
    pub engine: Box<dyn QueryEngine>,
}

impl GinHost {
    pub fn new() -> Self {
        let (tx, _rx) = unbounded();
        Self {
            engine: Box::new(CacheEngine::new(tx)),
        }
    }

    pub fn snapshot(&self) -> GinSnapshot {
        GinSnapshot {
            engine: self.engine.snapshot(),
        }
    }

    /// Evict the cached package entry for the package containing `path`.
    /// Called when a file tab is closed to free typed AST memory.
    pub fn evict_package_for(&self, path: &Path) {
        let path = path.normalize_diagnostic_path();
        self.engine.invalidate_package_cache_for(&path);
    }

    /// Upsert a file into the database.
    pub fn upsert_file(&mut self, path: PathBuf, contents: String) {
        let path = path.normalize_diagnostic_path();
        if !self.engine.contains(&path) {
            let _ = self.engine.add_file(path.clone());
        }
        self.engine.set_contents(&path, contents);
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
            if !self.engine.contains(p) {
                let _ = self.engine.add_file(p.clone());
            }
        }
        PackageInfo { file_paths }
    }
}
