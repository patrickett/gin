//! Suggest `use` statements for symbols defined elsewhere in the project or deps.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use flask::FlaskConfigHandle;

use crate::file_helpers::GinPackageExt;
use crate::folder_module::normalize_local_import_path;
use crate::import_merge::{
    ImportRoot, RawSuggestion, collect_member_imports, symbol_already_imported,
};
use crate::import_query::find_package_root;

/// A single import fix: a full `use …` line (no trailing newline).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSuggestion {
    pub use_line: String,
    pub replace_line: Option<u32>,
    pub delete_lines: Vec<u32>,
}

/// Find import paths that would bring `symbol` into scope at `file_path`.
///
/// Search order: same folder module, same package (local path), then declared
/// path dependencies. Results are deduplicated and omit symbols already imported
/// in `source`.
pub fn find_import_suggestions(
    file_path: &Path,
    symbol: &str,
    source: &str,
) -> Vec<ImportSuggestion> {
    let file_dir = match file_path.parent() {
        Some(d) => d,
        None => return Vec::new(),
    };

    let collected = collect_member_imports(source);
    if symbol_already_imported(symbol, &collected) {
        return Vec::new();
    }

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    // Same folder module (sibling `.gin` files).
    if crate::public_symbols::find_public_def(file_dir, symbol).is_some() {
        push_merged_import(
            &mut seen,
            &mut out,
            &collected,
            file_path,
            ImportRoot::CurrentModule,
            symbol,
        );
    }

    let dependencies = dependency_dirs(file_dir);

    // Same package, different folder module — but not symbols that live only in a
    // nested path-dependency package (those are imported as `use dep.Symbol`).
    if let Some(pkg_root) = find_package_root(file_path)
        && let Some(def_file) = pkg_root.find_public_def(symbol)
    {
        let def_dir = match def_file.parent() {
            Some(d) => d,
            None => return out,
        };
        let in_dependency_subpackage = dependencies
            .values()
            .any(|dep_dir| def_file.starts_with(dep_dir));
        if !in_dependency_subpackage
            && def_dir != file_dir
            && let Some(local_path) = local_import_path_for_module(file_dir, &pkg_root, def_dir)
            && normalize_local_import_path(file_dir, &pkg_root, Path::new(&local_path)).is_ok()
        {
            push_merged_import(
                &mut seen,
                &mut out,
                &collected,
                file_path,
                ImportRoot::LocalPath(local_path),
                symbol,
            );
        }
    }

    // Path dependencies from flask.jsonc.
    for (dep_name, dep_dir) in dependencies {
        if dep_dir.find_public_def(symbol).is_some() {
            push_merged_import(
                &mut seen,
                &mut out,
                &collected,
                file_path,
                ImportRoot::Dependency(dep_name),
                symbol,
            );
        }
    }

    out
}

fn push_merged_import(
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<ImportSuggestion>,
    collected: &[crate::import_merge::CollectedImport],
    file_path: &Path,
    root: ImportRoot,
    symbol: &str,
) {
    let raw = RawSuggestion {
        root,
        symbol: symbol.to_string(),
    };
    let merged = raw.merge(collected, Some(file_path));
    if seen.insert(merged.use_line.clone()) {
        out.push(ImportSuggestion {
            use_line: merged.use_line,
            replace_line: merged.replace_line,
            delete_lines: merged.delete_lines,
        });
    }
}

fn dependency_dirs(file_dir: &Path) -> HashMap<String, PathBuf> {
    let handle = match FlaskConfigHandle::load(file_dir) {
        Ok(h) => h,
        Err(_) => return HashMap::new(),
    };
    let cfg = handle.read();
    let config_dir = handle.source_dir();
    config_dir.resolve_flask_dependencies(&cfg.config)
}

/// Quoted import path (no surrounding quotes) from `from_dir` to folder module `target_dir`.
fn local_import_path_for_module(
    from_dir: &Path,
    pkg_root: &Path,
    target_dir: &Path,
) -> Option<String> {
    // Prefer a path relative to the importing file's directory.
    if let Some(rel) = path_relative(from_dir, target_dir)
        && normalize_local_import_path(from_dir, pkg_root, Path::new(&rel)).is_ok()
    {
        return Some(rel);
    }

    // Fall back to path from package root (e.g. `primitive/` from `io/`).
    if let Ok(rel) = target_dir.strip_prefix(pkg_root) {
        let components: Vec<_> = rel
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect();
        if !components.is_empty() {
            let mut from_root = components.join("/");
            if !from_root.ends_with('/') {
                from_root.push('/');
            }
            if normalize_local_import_path(from_dir, pkg_root, Path::new(&from_root)).is_ok() {
                return Some(from_root);
            }
        }
    }

    None
}

fn path_relative(from: &Path, to: &Path) -> Option<String> {
    use std::path::Component;

    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let mut i = 0usize;
    while i < from.len() && i < to.len() && from[i] == to[i] {
        i += 1;
    }
    if i == from.len() && i == to.len() {
        return Some("./".to_string());
    }
    let mut parts: Vec<String> = (0..from.len().saturating_sub(i))
        .map(|_| "..".to_string())
        .collect();
    for c in &to[i..] {
        if let Component::Normal(s) = c {
            parts.push(s.to_string_lossy().into_owned());
        }
    }
    let mut s = parts.join("/");
    if !s.ends_with('/') {
        s.push('/');
    }
    Some(s)
}
#[cfg(test)]
#[path = "../tests/import_suggest_tests.rs"]
mod tests;
