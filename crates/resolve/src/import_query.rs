use std::path::{Path, PathBuf};

use ast::{FileAst, HasSpanId, ImportSource};
use flask::{FlaskConfig, FlaskConfigHandle};
use parser::parse_source_full;

use crate::ParsedFile;
use crate::file_helpers;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// What a cursor position resolves to via `use` imports.
#[derive(Debug, Clone)]
pub enum ImportTarget {
    /// Cursor is on the root dependency name (e.g., `core` in `use core.true`).
    DepRoot { dep_name: String },
    /// Cursor is on a segment that names a public symbol from a dependency.
    DepSymbol { dep_name: String, symbol: String },
    /// Cursor is on a bare word in the body that matches an import's effective name.
    BodySymbol { dep_name: String, symbol: String },
}

// ---------------------------------------------------------------------------
// Default file reader (I/O-backed)
// ---------------------------------------------------------------------------

/// Read and parse a `.gin` file from disk.
///
/// Pass this as `file_reader` to [`resolve_symbol_hover`] and
/// [`resolve_symbol_def_span`] when you don't have a cached source (e.g. in
/// `ginc` or a simple tool). LSP clients should provide a Salsa-backed reader
/// instead to avoid redundant I/O.
pub fn default_file_reader(path: &Path) -> Option<ParsedFile> {
    let source = std::fs::read_to_string(path).ok()?;
    let output = parse_source_full(&source);
    Some(ParsedFile {
        path: path.to_path_buf(),
        source,
        output,
    })
}

// ---------------------------------------------------------------------------
// Public query API
// ---------------------------------------------------------------------------

/// Determine which part of a dotted path the cursor is on.
///
/// `0` = root (e.g. `core` in `core.true`), `1` = first segment (`true`),
/// `2` = second segment, etc.
pub fn part_index_in_dotted_path(span_text: &str, byte_in_span: usize) -> usize {
    let mut part = 0usize;
    for (i, ch) in span_text.char_indices() {
        if i >= byte_in_span {
            break;
        }
        if ch == '.' {
            part += 1;
        }
    }
    part
}

/// Resolve a cursor position to an import target, handling both phases:
///
/// **Phase 1:** cursor is directly inside a `use` statement's import path
/// (handles `ImportSource::Package` only — callers handle Local / LocalBundle
/// separately).
///
/// **Phase 2:** cursor is on a bare word in the body that matches an import's
/// effective name (e.g., `true` from `use core.true`).
///
/// Returns `None` if the cursor is not on an import-related symbol.
pub fn resolve_import_at(ast: &FileAst, source: &str, byte_pos: usize) -> Option<ImportTarget> {
    // Phase 1: cursor directly inside a `use` statement's import path.
    for import in ast.uses() {
        for mi in &import.0 {
            if let ImportSource::Package(mp) = &mi.source {
                let span_table = ast.span_table();
                let span = span_table.get(mp.span_id());
                if byte_pos < span.start || byte_pos > span.end {
                    continue;
                }

                let span_text = source.get(span.start..span.end).unwrap_or("");
                let byte_in_span = byte_pos.saturating_sub(span.start);
                let part = part_index_in_dotted_path(span_text, byte_in_span);

                if part == 0 {
                    return Some(ImportTarget::DepRoot {
                        dep_name: mp.root.as_str().to_string(),
                    });
                }

                let seg_idx = part.saturating_sub(1);
                if seg_idx >= mp.segments.len() {
                    return None;
                }

                let symbol = mp.segments[seg_idx].as_str().to_string();
                return Some(ImportTarget::DepSymbol {
                    dep_name: mp.root.as_str().to_string(),
                    symbol,
                });
            }
        }
    }

    // Phase 2: bare word matching an import's effective name.
    let word = ast
        .word_at_byte(byte_pos, source)
        .or_else(|| typeck::word_at_byte_offset(source, byte_pos));

    if let Some(word) = word {
        for import in ast.uses() {
            for mi in &import.0 {
                let imported_name = mi
                    .alias
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| mi.effective_name());
                if imported_name == word
                    && let ImportSource::Package(mp) = &mi.source
                    && mp.segments.len() == 1
                {
                    return Some(ImportTarget::BodySymbol {
                        dep_name: mp.root.as_str().to_string(),
                        symbol: mp.segments[0].as_str().to_string(),
                    });
                }
            }
        }
    }

    None
}

/// Resolve a dependency directory from any file path by loading `flask.jsonc`
/// from the file's parent directory and looking up the dependency by name.
pub fn resolve_dep_dir(file_path: &Path, dep_name: &str) -> Option<PathBuf> {
    let base_dir = file_path.parent()?;
    let handle = FlaskConfigHandle::load(base_dir).ok()?;
    let cfg = handle.read();
    let config_dir = handle.source_dir();
    let dep = cfg.config.dependencies().get(dep_name)?;
    match &dep.kind {
        flask::DependencyKind::Path { path } => Some(config_dir.join(path)),
        _ => None,
    }
}

/// Show hover information about a dependency root (e.g. hovering over `core`
/// in `use core.true`). Returns formatted markdown with name, description, version.
pub fn resolve_dep_hover(file_path: &Path, dep_name: &str) -> Option<String> {
    let dep_dir = resolve_dep_dir(file_path, dep_name)?;
    let dep_config = FlaskConfig::from_directory(&dep_dir)?;
    let name = dep_config.name();
    let version = dep_config.version();
    let description = dep_config.description().unwrap_or("");

    let mut text = format!("```gin\n{name}\n```");
    if !description.is_empty() {
        text.push_str(&format!("\n\n---\n\n{description}"));
    }
    text.push_str(&format!("\n\n---\n\nversion = {version}"));
    Some(text)
}

/// Resolve a symbol from a dependency (public definition) and return its
/// hover text.
///
/// `file_reader` reads + parses `.gin` files. Provide
/// [`default_file_reader`] for disk I/O, or a Salsa-backed reader in the LSP.
pub fn resolve_symbol_hover(
    file_path: &Path,
    dep_name: &str,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<String> {
    let dep_dir = resolve_dep_dir(file_path, dep_name)?;
    let def_file = file_helpers::find_public_def_in_package(&dep_dir, symbol)?;
    let parsed = file_reader(&def_file)?;
    let def_span = typeck::find_definition_span(&parsed.output.ast, symbol)?;
    typeck::hover_at(&parsed.source, &parsed.output.ast, def_span.start)
}

/// Resolve a symbol from a dependency and return its definition span (byte
/// range in the definition file) for goto-definition.
///
/// `file_reader` reads + parses `.gin` files. Provide
/// [`default_file_reader`] for disk I/O, or a Salsa-backed reader in the LSP.
pub fn resolve_symbol_def_span(
    file_path: &Path,
    dep_name: &str,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<std::ops::Range<usize>> {
    let dep_dir = resolve_dep_dir(file_path, dep_name)?;
    let def_file = file_helpers::find_public_def_in_package(&dep_dir, symbol)?;
    let parsed = file_reader(&def_file)?;
    typeck::find_definition_span(&parsed.output.ast, symbol)
}
