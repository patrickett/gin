//! Checking one document (or the package it belongs to) into flaws, warnings and holes, by the
//! same code `ginterpreter` and `ginc --check-new` use. Nothing here decides what is wrong.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use flask::MANIFEST_NAME;
use ginterpreter::check::{Checked, blank_source_range, check_opened_package, check_text};
use ginterpreter::flaw::Flaw;

/// The unit a document is checked as: a package it is in, or itself.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unit {
    Package(PathBuf),
    File(PathBuf),
}

/// What checking a document's unit produced.
pub struct Analysis {
    pub unit: Unit,
    pub checked: Checked,
    /// Files the package loaded (empty for a single file, or when loading failed).
    pub loaded: Vec<PathBuf>,
}

/// Canonical form of a path for comparisons; the path itself when it does not exist (yet).
pub fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// The nearest ancestor directory with a package manifest.
pub fn package_root(path: &Path) -> Option<PathBuf> {
    path.ancestors().skip(1).find(|dir| dir.join(MANIFEST_NAME).is_file()).map(Path::to_owned)
}

/// Check `path` whose current text is `text`. `buffers` are all open documents by canonical path;
/// a package reads them in place of the files on disk, so unsaved edits are checked.
pub fn analyze(path: &Path, text: &str, buffers: &BTreeMap<PathBuf, String>) -> Analysis {
    let path = canonical(path);
    if let Some(root) = package_root(&path) {
        let root = canonical(&root);
        let name = root.display().to_string();
        match package::Package::open(&root) {
            Ok(package) => {
                let overlay = Arc::new(buffers.clone());
                let package = package.with_overlay(Arc::clone(&overlay));
                let read = |p: &Path| overlay.get(&canonical(p)).cloned().or_else(|| std::fs::read_to_string(p).ok());
                let (checked, loaded) = check_opened_package(&name, &package, &read);
                let loaded: Vec<PathBuf> = loaded.iter().map(|p| canonical(p)).collect();
                // A file no module of the package reaches is checked on its own.
                if loaded.is_empty() || loaded.contains(&path) {
                    return Analysis { unit: Unit::Package(root), checked, loaded };
                }
            }
            Err(error) => {
                // A manifest that does not open is the document's problem to show.
                let mut checked = check_text(&path.display().to_string(), text);
                let message = format!("package manifest in {name}: {error}");
                checked.flaws.insert(0, unlocated_flaw(&path, &message));
                return Analysis { unit: Unit::File(path), checked, loaded: Vec::new() };
            }
        }
    }
    let checked = check_text(&path.display().to_string(), text);
    Analysis { unit: Unit::File(path), checked, loaded: Vec::new() }
}

fn unlocated_flaw(path: &Path, message: &str) -> Flaw {
    let mut flaw = ginterpreter::session::message_flaw(message);
    flaw.file = path.display().to_string();
    flaw.stage = "check";
    flaw
}

/// The text with every declaration that has a flaw blanked (newlines kept, so offsets hold), and
/// its check: what hover can inspect when the file as a whole does not check.
pub fn without_flawed(file: &str, text: &str, checked: &Checked) -> Checked {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for flaw in &checked.flaws {
        if let Some(span) = &flaw.span {
            if let Some(decl) = checked.decls.iter().find(|d| d.start <= span.byte_start && span.byte_end <= d.end) {
                spans.push((decl.start, decl.end));
            }
        }
    }
    spans.sort_unstable();
    spans.dedup();
    let mut out = text.to_owned();
    for (start, end) in spans {
        if end <= out.len() && out.is_char_boundary(start) && out.is_char_boundary(end) {
            out = blank_source_range(&out, start, end);
        }
    }
    check_text(file, &out)
}
