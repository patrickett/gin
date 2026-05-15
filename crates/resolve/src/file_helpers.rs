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
///
/// First searches the platform-filtered AST. When that fails (e.g. a
/// `#[os({ linux })]` tag on a macOS host), falls back to a source-text
/// search that checks whether the symbol appears as a top-level
/// declaration (`Tag is …`, `Tag has …`, `name:` or `name :=`).
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
        // Fallback: platform-gated symbols (e.g. #[os({ linux })] on macOS)
        // are dropped by collapse_tags_for_platform and won't appear in
        // the AST. Scan the raw source for a top-level declaration.
        if source_has_top_level_symbol(&source, symbol_name) {
            return Some(path.clone());
        }
    }
    None
}

/// Check whether `symbol` appears as a top-level declaration in `source`.
///
/// Looks for patterns like `Tag is …`, `Tag has …`, `name: …`, or
/// `name := …` at the start of a logical line (possibly preceded by
/// `#[…]` attributes and whitespace).
fn source_has_top_level_symbol(source: &str, symbol: &str) -> bool {
    let mut in_attrs = true;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("--") {
            continue;
        }
        // Skip attribute lines that precede a declaration.
        if in_attrs && (trimmed.starts_with("#[") || trimmed.starts_with("@[")) {
            continue;
        }
        in_attrs = false;
        // Check for tag declaration: `Tag is …`, `Tag has …`,
        // `Tag(params) is …`, `Tag[params] is …`.
        if let Some(rest) = trimmed.strip_prefix(symbol) {
            let rest = rest.trim_start();
            if rest.starts_with("is ") || rest.starts_with("has ") || rest == "is" || rest == "has"
            {
                return true;
            }
            if rest.starts_with('(') || rest.starts_with('[') {
                return true;
            }
        }
        // Check for bind declaration: `name: …` or `name := …`.
        if let Some(rest) = trimmed.strip_prefix(symbol) {
            let rest = rest.trim_start();
            if rest.starts_with(':') || rest.starts_with(":=") {
                return true;
            }
        }
    }
    false
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
