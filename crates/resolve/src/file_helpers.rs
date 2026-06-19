//! Gin package-directory operations — discovering `.gin` files, listing public
//! symbols, finding definitions, and resolving path dependencies.
//!
//! These are exposed via the [`GinPackageExt`] extension trait so callers can
//! use them as `package_dir.collect_gin_files()` instead of passing raw paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use flask::{DependencyKind, FlaskConfig, FlaskPathExt, PACKAGE_CONFIG_NAME};
use parser::gin_walk::GinPathExt;

/// Extension trait adding Gin-package queries to [`Path`].
///
/// The receiver is always a directory path — a Gin package root or folder module.
pub trait GinPackageExt {
    /// Collect `.gin` file paths under this directory, honoring `.gitignore`.
    ///
    /// If the directory is a folder module (contains `flask.jsonc`), all package
    /// `.gin` files are collected recursively. Otherwise, the directory is scanned
    /// recursively. A single file returns itself.
    fn collect_gin_files(&self) -> Vec<PathBuf>;

    /// Recursively collect `.gin` file paths under this directory, honoring `.gitignore`.
    fn collect_gin_files_recursive(&self) -> Vec<PathBuf>;

    /// List all public (exported) symbol names in any `.gin` file under this directory.
    fn list_public_symbols(&self) -> Vec<String>;

    /// Check whether `symbol_name` is a public (exported) definition in any
    /// `.gin` file under this directory.
    ///
    /// First searches parsed AST defs/tags. When that fails, falls back to a
    /// source-text search that checks whether the symbol appears as a top-level
    /// declaration (`Tag is …`, `Tag has …`, `name:` or `name :=`).
    fn find_public_def(&self, symbol_name: &str) -> Option<PathBuf>;

    /// Whether `symbol_name` is a public definition under this directory.
    fn check_public_def(&self, symbol_name: &str) -> bool;

    /// Resolve path dependencies from a flask config relative to this package root.
    fn resolve_flask_dependencies(&self, config: &FlaskConfig) -> HashMap<String, PathBuf>;

    /// Path dependencies for the package containing this path (walks up to `flask.jsonc`).
    fn flask_path_dependencies(&self) -> HashMap<String, PathBuf>;

    /// Collect `.gin` paths to scan for public symbols under this directory.
    fn gin_paths_for_symbol_lookup(&self) -> Vec<PathBuf>;
}

impl GinPackageExt for Path {
    fn collect_gin_files(&self) -> Vec<PathBuf> {
        let root = self;
        if root.is_dir() {
            if root.join(PACKAGE_CONFIG_NAME).is_file() {
                root.collect_gin_files_under()
            } else {
                self.collect_gin_files_recursive()
            }
        } else {
            vec![root.to_path_buf()]
        }
    }

    fn collect_gin_files_recursive(&self) -> Vec<PathBuf> {
        self.collect_gin_files_under()
    }

    fn list_public_symbols(&self) -> Vec<String> {
        crate::public_symbols::list_public_symbols(self)
    }

    fn find_public_def(&self, symbol_name: &str) -> Option<PathBuf> {
        crate::public_symbols::find_public_def(self, symbol_name, true)
    }

    fn check_public_def(&self, symbol_name: &str) -> bool {
        self.find_public_def(symbol_name).is_some()
    }

    fn resolve_flask_dependencies(&self, config: &FlaskConfig) -> HashMap<String, PathBuf> {
        let package_root = self;
        let mut dependencies = HashMap::new();
        for (name, dep) in config.dependencies() {
            if let DependencyKind::Path { path: dep_path } = &dep.kind {
                dependencies.insert(name.clone(), package_root.join(dep_path));
            }
        }
        dependencies
    }

    fn flask_path_dependencies(&self) -> HashMap<String, PathBuf> {
        let file_dir = match self.parent() {
            Some(d) => d,
            None => return HashMap::new(),
        };
        let Some((config, package_root)) = FlaskConfig::find_package_config(file_dir) else {
            return HashMap::new();
        };
        package_root.resolve_flask_dependencies(&config)
    }

    fn gin_paths_for_symbol_lookup(&self) -> Vec<PathBuf> {
        if self.join(PACKAGE_CONFIG_NAME).is_file() {
            self.collect_gin_files_under()
        } else {
            self.list_package_gin_files()
        }
    }
}
