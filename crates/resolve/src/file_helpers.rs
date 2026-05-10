use std::collections::HashMap;
use std::path::{Path, PathBuf};

use flask::{DependencyKind, FlaskConfig, PACKAGE_CONFIG_NAME};
use internment::Intern;
use parser::parse_source_full;

// ---------------------------------------------------------------------------
// File-system helpers
// ---------------------------------------------------------------------------

pub fn is_folder_module_dir(path: &Path) -> bool {
    path.is_dir() && path.join(PACKAGE_CONFIG_NAME).is_file()
}

/// Collect `.gin` file paths under `root`, skipping `target/` directories.
///
/// If `root` is a folder module (contains `flask.jsonc`), only immediate
/// `*.gin` files from the package manifest are returned. Otherwise, the
/// directory is scanned recursively.
pub fn collect_gin_files(root: &Path) -> Vec<PathBuf> {
    if root.is_dir() {
        if root.join(PACKAGE_CONFIG_NAME).is_file() {
            flask::list_package_gin_files(root)
        } else {
            collect_gin_files_recursive(root)
        }
    } else {
        vec![root.to_path_buf()]
    }
}

/// Recursively collect `.gin` file paths under `dir`, skipping `target/`
/// directories.
pub fn collect_gin_files_recursive(dir: &Path) -> Vec<PathBuf> {
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

/// List all public (exported) symbol names in any `.gin` file under
/// `package_dir`, including both defs and tags.
pub fn list_public_symbols(package_dir: &Path) -> Vec<String> {
    let paths = flask::list_package_gin_files(package_dir);
    let mut symbols = Vec::new();
    for path in &paths {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let output = parse_source_full(&source);
        for (key, _) in &output.ast.defs {
            if !output.ast.private_defs.contains(key) {
                symbols.push(key.as_str().to_string());
            }
        }
        for key in output.ast.tags.keys() {
            if !output.ast.private_tags.contains(key) {
                symbols.push(key.as_str().to_string());
            }
        }
    }
    symbols.sort();
    symbols.dedup();
    symbols
}

/// Check whether `symbol_name` is a public (exported) definition in any
/// `.gin` file under `package_dir`.
pub fn find_public_def_in_package(package_dir: &Path, symbol_name: &str) -> Option<PathBuf> {
    let paths = flask::list_package_gin_files(package_dir);
    let target = Intern::<String>::from_ref(symbol_name);
    for path in &paths {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let output = parse_source_full(&source);
        if !output.ast.private_defs.contains(&target) && output.ast.defs.contains_key(&target) {
            return Some(path.clone());
        }
        if !output.ast.private_tags.contains(&target) && output.ast.tags.contains_key(&target) {
            return Some(path.clone());
        }
    }
    None
}

pub fn check_public_def_in_package(package_dir: &Path, symbol_name: &str) -> bool {
    find_public_def_in_package(package_dir, symbol_name).is_some()
}

pub fn resolve_flask_path_dependencies(
    config: &FlaskConfig,
    config_dir: &Path,
) -> HashMap<String, PathBuf> {
    let mut dependencies = HashMap::new();
    for (name, dep) in config.dependencies() {
        if let DependencyKind::Path { path: dep_path } = &dep.kind {
            dependencies.insert(name.clone(), config_dir.join(dep_path));
        }
    }
    dependencies
}
