//! Gin package-directory operations — discovering `.gin` files, listing public
//! symbols, finding definitions, and resolving path dependencies.
//!
//! These are exposed via the [`GinPackageExt`] extension trait so callers can
//! use them as `package_dir.collect_gin_files()` instead of passing raw paths.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::module_inventory::ModuleInventory;
use crate::module_loader::{ModuleLoader, ParsedModuleCache};
use flask::{DependencyKind, FlaskConfig};
use parser::query::SourceParseExt;

/// Extension trait adding Gin-package queries to [`Path`].
///
/// The receiver is always a directory path — a Gin package root or folder module.
pub trait GinPackageExt {
    /// Collect `.gin` file paths under this directory, honoring `.gitignore`.
    ///
    /// If the directory is a folder module (contains `mod.flask`), all package
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
    /// declaration (`Tag is …`, `Tag has …`, `name:` or `name :=`.
    fn find_public_def(&self, symbol_name: &str) -> Option<PathBuf>;

    /// Resolve path dependencies from a flask config relative to this package root.
    fn resolve_flask_dependencies(&self, config: &FlaskConfig) -> HashMap<String, PathBuf>;

    /// Path dependencies for the package containing this path (walks up to `mod.flask`).
    fn flask_path_dependencies(&self) -> HashMap<String, PathBuf>;
}

impl GinPackageExt for Path {
    fn collect_gin_files(&self) -> Vec<PathBuf> {
        let root = self;
        if root.is_dir() {
            self.collect_gin_files_recursive()
        } else {
            vec![root.to_path_buf()]
        }
    }

    fn collect_gin_files_recursive(&self) -> Vec<PathBuf> {
        let inventory = ModuleInventory::discover();
        let mut loader = ModuleLoader::with_cache(inventory, [], ParsedModuleCache::default());
        let mut files = Vec::new();
        let mut queue = VecDeque::from([self.to_path_buf()]);

        while let Some(dir) = queue.pop_front() {
            files.extend(loader.load_module(&dir));

            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && (path == self || !FlaskConfig::manifest_path(&path).is_file()) {
                    queue.push_back(path);
                }
            }
        }

        files.sort();
        files.dedup();
        files
    }

    fn list_public_symbols(&self) -> Vec<String> {
        let mut symbols = Vec::new();
        for path in self.collect_gin_files_recursive() {
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            let output = source.parse_source_full();
            symbols.extend(
                output
                    .ast
                    .public_symbol_names()
                    .into_iter()
                    .map(|name| name.as_str().to_string()),
            );
        }
        symbols.sort();
        symbols.dedup();
        symbols
    }

    fn find_public_def(&self, symbol_name: &str) -> Option<PathBuf> {
        crate::public_symbols::find_public_def_in_paths(
            self.collect_gin_files_recursive(),
            symbol_name,
        )
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
}
