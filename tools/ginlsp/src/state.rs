use crossbeam_channel::unbounded;
use database::input_database::InputDatabase;
use database::{set_file_contents, Db, File};
use std::path::{Path, PathBuf};

pub struct DocumentState {
    pub source: String,
    pub file: File,
}

pub struct JsonDocumentState {
    pub source: String,
}

#[derive(Clone)]
pub struct GinSnapshot {
    pub db: InputDatabase,
}

/// Information about the files belonging to a Gin package.
pub struct PackageInfo {
    /// All `.gin` files discovered in the package source directory.
    pub files: Vec<File>,
    /// The root directory of the package (where `flask.jsonc` lives).
    #[allow(dead_code)]
    pub root: PathBuf,
}

pub struct GinHost {
    pub db: InputDatabase,
}

impl GinHost {
    pub fn new() -> Self {
        let (tx, _rx) = unbounded();
        Self {
            db: InputDatabase::new(tx),
        }
    }

    pub fn snapshot(&self) -> GinSnapshot {
        GinSnapshot {
            db: self.db.clone(),
        }
    }

    /// Upsert a file into the database.
    pub fn upsert_file(&mut self, path: PathBuf, contents: String) -> Option<File> {
        match self.db.input(path) {
            Ok(file) => {
                set_file_contents(&mut self.db, file, contents);
                Some(file)
            }
            Err(_) => None,
        }
    }

    /// Discover all `.gin` files under `dir`, load them into the database,
    /// and return a [`PackageInfo`] describing the package.
    ///
    /// This mirrors the file-collection logic that `ginc` uses so that
    /// the LSP sees the same set of files as the CLI.
    pub fn load_package(&mut self, dir: &Path) -> PackageInfo {
        let file_paths = resolve::collect_gin_files(dir);
        let files = file_paths
            .iter()
            .filter_map(|p| self.db.input(p.clone()).ok())
            .collect();
        PackageInfo {
            files,
            root: dir.to_path_buf(),
        }
    }
}
