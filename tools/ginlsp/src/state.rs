use crossbeam_channel::unbounded;
use database::SalsaQueryEngine;
use database::QueryEngine;
use std::path::{Path, PathBuf};

pub struct DocumentState {
    pub source: String,
    pub file_path: PathBuf,
}

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
    /// The root directory of the package (where `flask.jsonc` lives).
    #[allow(dead_code)]
    pub root: PathBuf,
}

pub struct GinHost {
    pub engine: Box<dyn QueryEngine>,
}

impl GinHost {
    pub fn new() -> Self {
        let (tx, _rx) = unbounded();
        Self {
            engine: Box::new(SalsaQueryEngine::new(tx)),
        }
    }

    pub fn snapshot(&self) -> GinSnapshot {
        GinSnapshot {
            engine: self.engine.snapshot(),
        }
    }

    /// Upsert a file into the database.
    pub fn upsert_file(&mut self, path: PathBuf, contents: String) {
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
        let file_paths = resolve::collect_gin_files(dir);
        for p in &file_paths {
            let _ = self.engine.add_file(p.clone());
        }
        PackageInfo {
            file_paths,
            root: dir.to_path_buf(),
        }
    }
}
