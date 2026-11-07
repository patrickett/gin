//! Go-to-definition and workspace symbols, over `ast` and the package layout. Neither
//! needs the checker: a name is found where a module declares it. The checker keeps no
//! reference table, so references and renames are not offered.

use std::path::{Path, PathBuf};

use lsp_types::{Location, SymbolInformation, Url};
use ast::SourceFile;

use crate::analysis::{canonical, package_root};
use crate::position::LineIndex;
use crate::symbols::{declared, imports};

/// File contents by path: an open buffer, else the file.
pub type Read<'a> = &'a dyn Fn(&Path) -> Option<String>;

const FILES_MAX: usize = 5000;
const FILE_BYTES_MAX: u64 = 4 * 1024 * 1024;
const RESULTS_MAX: usize = 500;

fn parse(text: &str) -> Option<SourceFile> {
    parser::parse(text).ok().map(|output| output.file)
}

/// The immediate `.gin` files of `dir`, sorted: the files of one folder module.
pub fn module_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "gin") && p.is_file())
        .collect();
    files.sort();
    files
}

fn location_in(path: &Path, text: &str, start: usize, end: usize) -> Option<Location> {
    let uri = Url::from_file_path(path).ok()?;
    Some(Location { uri, range: LineIndex::new(text).range(text, start, end) })
}

/// Where `name` is declared at the top level of any file of the module in `dir`.
fn find_in_module(dir: &Path, name: &str, read: Read<'_>, skip: Option<&Path>) -> Option<(PathBuf, String, usize, usize)> {
    for path in module_files(dir) {
        if skip.is_some_and(|s| canonical(s) == canonical(&path)) {
            continue;
        }
        let Some(text) = read(&path) else { continue };
        let Some(file) = parse(&text) else { continue };
        if let Some(d) = declared(&file).into_iter().find(|d| d.name == name) {
            return Some((path, text.clone(), d.name_start, d.name_end));
        }
    }
    None
}

/// The directory of the module a dotted `use` path names, from the package `from` is in.
fn module_dir(from: &Path, module: &[String]) -> Option<PathBuf> {
    let root = package_root(&canonical(from))?;
    let package = package::Package::open(&root).ok()?;
    let (head, rest) = module.split_first()?;
    let base = if head == package.name() {
        package.root().to_owned()
    } else {
        match package.dependency(head)? {
            package::DependencyRoot::Path(path) => path.clone(),
            package::DependencyRoot::Unsupported(_) => return None,
        }
    };
    let dir = rest.iter().fold(base, |dir, segment| dir.join(segment));
    dir.is_dir().then_some(dir)
}

/// The declaration of the name at `offset` in `path`'s text: in the same file, in the other files
/// of its module, or through a `use` of it.
pub fn definition(path: &Path, text: &str, offset: usize, read: Read<'_>) -> Option<Location> {
    let (start, end) = crate::hover::word_at(text, offset)?;
    let word = &text[start..end];
    let file = parse(text)?;
    if let Some(d) = declared(&file).into_iter().find(|d| d.name == word) {
        return location_in(path, text, d.name_start, d.name_end);
    }
    if let Some(dir) = path.parent() {
        if let Some((found, found_text, s, e)) = find_in_module(dir, word, read, Some(path)) {
            return location_in(&found, &found_text, s, e);
        }
    }
    for import in imports(&file) {
        let Some((exported, _)) = import.members.iter().find(|(_, bound)| bound == word) else { continue };
        let Some(dir) = module_dir(path, &import.module) else { continue };
        if let Some((found, found_text, s, e)) = find_in_module(&dir, exported, read, None) {
            return location_in(&found, &found_text, s, e);
        }
    }
    None
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if out.len() >= FILES_MAX {
            return;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "gin") && std::fs::metadata(&path).is_ok_and(|m| m.len() <= FILE_BYTES_MAX) {
            out.push(path);
        }
    }
}

/// Top-level declarations in the workspace roots whose name contains `query` (case-insensitive).
#[allow(deprecated)]
pub fn workspace_symbols(roots: &[PathBuf], query: &str, read: Read<'_>) -> Vec<SymbolInformation> {
    let query = query.to_lowercase();
    let mut files = Vec::new();
    for root in roots {
        walk(root, &mut files);
    }
    let mut out = Vec::new();
    for path in files {
        let Some(text) = read(&path) else { continue };
        let Some(file) = parse(&text) else { continue };
        let index = LineIndex::new(&text);
        let container = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).map(str::to_owned);
        for d in declared(&file) {
            if !d.name.to_lowercase().contains(&query) {
                continue;
            }
            let Ok(uri) = Url::from_file_path(&path) else { continue };
            out.push(SymbolInformation {
                name: d.name,
                kind: d.kind,
                tags: None,
                deprecated: None,
                location: Location { uri, range: index.range(&text, d.name_start, d.name_end) },
                container_name: container.clone(),
            });
            if out.len() >= RESULTS_MAX {
                return out;
            }
        }
    }
    out
}
