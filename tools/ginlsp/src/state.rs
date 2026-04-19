use ast::FileAst;
use crossbeam_channel::unbounded;
use database::input_database::InputDatabase;
use database::{Db, File};
use parser::parse_from_str;
use salsa::Setter;
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

impl GinSnapshot {
    /// Parse a file using the pure parser (no Salsa tracking).
    pub fn parse(&self, file: File) -> FileAst {
        let source = file.contents(&self.db);
        parse_from_str(source)
    }
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
                file.set_contents(&mut self.db).to(contents);
                Some(file)
            }
            Err(_) => None,
        }
    }

    /// Discover all `.gin` files under `dir`, load them into the database,
    /// and return a [`PackageInfo`] describing the package.
    ///
    /// This mirrors the file-collection logic that `begin build` uses so that
    /// the LSP sees the same set of files as the CLI.
    pub fn load_package(&mut self, dir: &Path) -> PackageInfo {
        let paths = collect_gin_files_recursive(dir);
        let files = paths
            .iter()
            .filter_map(|p| self.db.input(p.clone()).ok())
            .collect();
        PackageInfo {
            files,
            root: dir.to_path_buf(),
        }
    }
}

/// Collect all `.gin` file paths in a directory recursively, skipping `target/`.
fn collect_gin_files_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            files.extend(collect_gin_files_recursive(&path));
        } else if path.extension().is_some_and(|ext| ext == "gin") {
            files.push(path);
        }
    }

    files
}
