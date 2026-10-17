//! Public symbol discovery for Package roots and Folder modules.

use std::path::{Path, PathBuf};

use internment::Intern;
use parser::query::SourceParseExt;

pub fn find_public_def_in_module(root: &Path, symbol_name: &str) -> Option<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "gin"))
        .collect();
    paths.sort();
    find_public_def_in_paths(paths, symbol_name)
}

pub(crate) fn find_public_def_in_paths(paths: Vec<PathBuf>, symbol_name: &str) -> Option<PathBuf> {
    let target = Intern::<String>::from_ref(symbol_name);
    for path in paths {
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let output = source.parse_source_full();
        if output.ast.contains_public_symbol(&target) {
            return Some(path);
        }
    }
    None
}
