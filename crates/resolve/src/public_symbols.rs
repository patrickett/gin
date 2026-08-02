//! Public symbol discovery for Package roots and Folder modules.

use std::path::{Path, PathBuf};

use internment::Intern;
use parser::query::SourceParseExt;

use crate::GinPackageExt;

/// Find a public definition; uses source-text fallback when `text_fallback` is true (package scope).
pub fn find_public_def(root: &Path, symbol_name: &str, text_fallback: bool) -> Option<PathBuf> {
    find_public_def_in_paths(
        root.gin_paths_for_symbol_lookup(),
        symbol_name,
        text_fallback,
    )
}

pub fn find_public_def_in_module(root: &Path, symbol_name: &str) -> Option<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "gin"))
        .collect();
    paths.sort();
    find_public_def_in_paths(paths, symbol_name, false)
}

fn find_public_def_in_paths(
    paths: Vec<PathBuf>,
    symbol_name: &str,
    text_fallback: bool,
) -> Option<PathBuf> {
    let target = Intern::<String>::from_ref(symbol_name);
    for path in paths {
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let output = source.parse_source_full();
        if !output.ast.private_defs.contains(&target) && output.ast.defs.contains_key(&target) {
            return Some(path);
        }
        if !output.ast.private_tags.contains(&target) && output.ast.tags.contains_key(&target) {
            return Some(path);
        }
        if text_fallback && source_has_top_level_symbol(&source, symbol_name) {
            return Some(path);
        }
    }
    None
}

/// List public symbol names under `root`.
pub fn list_public_symbols(root: &Path) -> Vec<String> {
    let mut symbols = Vec::new();
    for path in root.gin_paths_for_symbol_lookup() {
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let output = source.parse_source_full();
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

fn source_has_top_level_symbol(source: &str, symbol: &str) -> bool {
    let mut in_attrs = true;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("--") {
            continue;
        }
        if in_attrs && (trimmed.starts_with("#[") || trimmed.starts_with("@[")) {
            continue;
        }
        in_attrs = false;
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
        if let Some(rest) = trimmed.strip_prefix(symbol) {
            let rest = rest.trim_start();
            if rest.starts_with(':') || rest.starts_with(":=") {
                return true;
            }
        }
    }
    false
}
