use std::path::{Path, PathBuf};

use ast::{FileAst, ImportSource};
use flask::{FlaskConfig, FlaskConfigHandle};
use parser::parse_source_full;

use crate::ParsedFile;
use crate::file_helpers;

#[derive(Debug, Clone)]
pub enum ImportTarget {
    DepRoot {
        dep_name: String,
    },
    DepSymbol {
        dep_name: String,
        symbol: String,
    },
    BodySymbol {
        dep_name: String,
        symbol: String,
    },
    LocalBundleSymbol {
        local_path: PathBuf,
        symbol: String,
    },
    /// Symbol imported from the current module (same package, no dep prefix),
    /// e.g. `use Str, Int, Byte`
    CurrentModuleSymbol {
        symbol: String,
    },
}

pub fn default_file_reader(path: &Path) -> Option<ParsedFile> {
    let source = std::fs::read_to_string(path).ok()?;
    let output = parse_source_full(&source);
    Some(ParsedFile {
        path: path.to_path_buf(),
        source,
        output,
    })
}

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

            if let ImportSource::LocalBundle(b) = &mi.source {
                let local_path = match &b.local_path {
                    Some(p) => p.clone(),
                    None => continue,
                };

                let span_table = ast.span_table();
                for member in &b.members {
                    let mspan = span_table.get(member.span);
                    if byte_pos >= mspan.start && byte_pos <= mspan.end {
                        let symbol = member
                            .alias
                            .as_ref()
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| member.export.to_string());
                        return Some(ImportTarget::LocalBundleSymbol { local_path, symbol });
                    }
                }
            }

            if let ImportSource::CurrentModule { member } = &mi.source {
                let span_table = ast.span_table();
                let mspan = span_table.get(member.span);
                if byte_pos >= mspan.start && byte_pos <= mspan.end {
                    let symbol = member
                        .alias
                        .as_ref()
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| member.export.to_string());
                    return Some(ImportTarget::CurrentModuleSymbol { symbol });
                }
            }
        }
    }

    // Phase 2: bare word matching an import's effective name.
    let word = ast
        .word_at_byte(byte_pos, source)
        .or_else(|| ast::word_at_byte_offset(source, byte_pos));

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

                // Check LocalBundle members for body-word matches.
                if let ImportSource::LocalBundle(b) = &mi.source {
                    for member in &b.members {
                        let member_name = member.alias.unwrap_or(member.export).to_string();
                        if member_name == word {
                            return match &b.local_path {
                                Some(lp) => Some(ImportTarget::LocalBundleSymbol {
                                    local_path: lp.clone(),
                                    symbol: member.export.to_string(),
                                }),
                                None => Some(ImportTarget::DepSymbol {
                                    dep_name: b.root.to_string(),
                                    symbol: member.export.to_string(),
                                }),
                            };
                        }
                    }
                }

                if let ImportSource::CurrentModule { member } = &mi.source {
                    let member_name = member.alias.unwrap_or(member.export).to_string();
                    if member_name == word {
                        return Some(ImportTarget::CurrentModuleSymbol {
                            symbol: member.export.to_string(),
                        });
                    }
                }
            }
        }
    }

    None
}

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

pub fn resolve_symbol_hover(
    file_path: &Path,
    dep_name: &str,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<String> {
    let dep_dir = resolve_dep_dir(file_path, dep_name)?;
    let def_file = file_helpers::find_public_def_in_package(&dep_dir, symbol)?;
    let mut parsed = file_reader(&def_file)?;
    let def_span = ast::hover::definition_span(&parsed.output.ast, symbol)?;
    let analysis = ast::resolve_types(&parsed.output.ast, std::slice::from_ref(&parsed.output.ast));
    ast::populate_ast_types(&mut parsed.output.ast, &analysis);
    ast::hover::hover_at(
        &parsed.source,
        &parsed.output.ast,
        &analysis,
        def_span.start,
    )
}

pub fn resolve_symbol_def_span(
    file_path: &Path,
    dep_name: &str,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<std::ops::Range<usize>> {
    let dep_dir = resolve_dep_dir(file_path, dep_name)?;
    let def_file = file_helpers::find_public_def_in_package(&dep_dir, symbol)?;
    let parsed = file_reader(&def_file)?;
    ast::hover::definition_span(&parsed.output.ast, symbol)
}

fn resolve_local_import_target(
    file_path: &Path,
    local_import_path: &Path,
    symbol: &str,
) -> Option<(PathBuf, ast::FileAst)> {
    let base_dir = file_path.parent()?;
    let resolved = base_dir.join(local_import_path);
    let resolved: PathBuf = resolved.components().collect();

    let (target_file, file_reader) = if resolved.is_dir() {
        let def_file = file_helpers::find_public_def_in_package(&resolved, symbol)?;
        let parsed = default_file_reader(&def_file)?;
        (def_file, parsed)
    } else {
        let gin_file = if resolved.is_file() && resolved.extension().is_some_and(|e| e == "gin") {
            resolved.clone()
        } else {
            resolved.with_extension("gin")
        };
        let parent = gin_file.parent()?;
        let def_file = file_helpers::find_public_def_in_package(parent, symbol)?;
        let parsed = default_file_reader(&def_file)?;
        (def_file, parsed)
    };
    Some((target_file, file_reader.output.ast))
}

pub fn resolve_local_symbol_hover(
    file_path: &Path,
    local_import_path: &Path,
    symbol: &str,
    _file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<String> {
    let (target_file, mut ast) = resolve_local_import_target(file_path, local_import_path, symbol)?;
    let source = std::fs::read_to_string(&target_file).ok()?;
    let def_span = ast::hover::definition_span(&ast, symbol)?;
    let analysis = ast::resolve_types(&ast, std::slice::from_ref(&ast));
    ast::populate_ast_types(&mut ast, &analysis);
    ast::hover::hover_at(&source, &ast, &analysis, def_span.start)
}

pub fn resolve_local_symbol_def_span(
    file_path: &Path,
    local_import_path: &Path,
    symbol: &str,
    _file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<std::ops::Range<usize>> {
    let (_target_file, ast) = resolve_local_import_target(file_path, local_import_path, symbol)?;
    ast::hover::definition_span(&ast, symbol)
}

/// Walk up from `file_path` to find the package root (containing `flask.jsonc`).
pub fn find_package_root(file_path: &Path) -> Option<PathBuf> {
    let mut search = file_path.parent()?.to_path_buf();
    loop {
        let candidate = search.join(flask::PACKAGE_CONFIG_NAME);
        if candidate.exists() {
            return Some(search);
        }
        if !search.pop() {
            return None;
        }
    }
}

/// Resolve hover for a `use Symbol` import (current module, same package).
/// Finds the sibling `.gin` file where `symbol` is publicly defined and returns
/// its hover markdown.
pub fn resolve_current_module_hover(
    file_path: &Path,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<String> {
    let pkg_root = find_package_root(file_path)?;
    let def_file = file_helpers::find_public_def_in_package(&pkg_root, symbol)?;
    let mut parsed = file_reader(&def_file)?;
    let def_span = ast::hover::definition_span(&parsed.output.ast, symbol)?;
    let analysis = ast::resolve_types(&parsed.output.ast, std::slice::from_ref(&parsed.output.ast));
    ast::populate_ast_types(&mut parsed.output.ast, &analysis);
    ast::hover::hover_at(
        &parsed.source,
        &parsed.output.ast,
        &analysis,
        def_span.start,
    )
}

/// Resolve the definition span for a `use Symbol` import (current module).
pub fn resolve_current_module_def_span(
    file_path: &Path,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<std::ops::Range<usize>> {
    let pkg_root = find_package_root(file_path)?;
    let def_file = file_helpers::find_public_def_in_package(&pkg_root, symbol)?;
    let parsed = file_reader(&def_file)?;
    ast::hover::definition_span(&parsed.output.ast, symbol)
}
