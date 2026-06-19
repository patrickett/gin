use std::path::{Path, PathBuf};

use ast::source::SourceExt;
use ast::{FileAst, HasSpanId, ImportSource};
use flask::{FlaskConfig, FlaskConfigHandle};

use crate::ParsedFile;

#[derive(Debug, Clone)]
pub enum ImportTarget {
    DepRoot {
        dep_name: String,
    },
    DepSymbol {
        dep_name: String,
        symbol: String,
    },
    /// `use dep.folder.(sym)` — symbol inside a dependency folder module.
    DepBundleMember {
        dep_name: String,
        path_segments: Vec<String>,
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
    for import in &ast.uses {
        for mi in &import.0 {
            if let ImportSource::Package(mp) = &mi.source {
                let span_table = &ast.span_table;
                let span = span_table.get(mp.span_id());
                if byte_pos < span.start() || byte_pos > span.end() {
                    continue;
                }

                let span_text = source.get(span.start()..span.end()).unwrap_or("");
                let byte_in_span = byte_pos.saturating_sub(span.start());
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
                let span_table = &ast.span_table;
                for member in &b.members {
                    let mspan = span_table.get(member.span);
                    if byte_pos >= mspan.start() && byte_pos <= mspan.end() {
                        let symbol = member
                            .alias
                            .as_ref()
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| member.export.to_string());
                        if let Some(local_path) = &b.local_path {
                            return Some(ImportTarget::LocalBundleSymbol {
                                local_path: local_path.clone(),
                                symbol,
                            });
                        }
                        return Some(ImportTarget::DepBundleMember {
                            dep_name: b.root.to_string(),
                            path_segments: b.path_segments.iter().map(|s| s.to_string()).collect(),
                            symbol,
                        });
                    }
                }

                if b.local_path.is_none() && !b.path_segments.is_empty() {
                    let span = span_table.get(b.span_id());
                    if byte_pos < span.start() || byte_pos > span.end() {
                        continue;
                    }
                    let span_text = source.get(span.start()..span.end()).unwrap_or("");
                    let byte_in_span = byte_pos.saturating_sub(span.start());
                    let part = part_index_in_dotted_path(span_text, byte_in_span);
                    if part == 0 {
                        return Some(ImportTarget::DepRoot {
                            dep_name: b.root.to_string(),
                        });
                    }
                    let seg_idx = part.saturating_sub(1);
                    if seg_idx < b.path_segments.len() {
                        return Some(ImportTarget::DepSymbol {
                            dep_name: b.root.to_string(),
                            symbol: b.path_segments[seg_idx].to_string(),
                        });
                    }
                }
            }

            if let ImportSource::CurrentModule { member } = &mi.source {
                let span_table = &ast.span_table;
                let mspan = span_table.get(member.span);
                if byte_pos >= mspan.start() && byte_pos <= mspan.end() {
                    let symbol = member
                        .alias
                        .as_ref()
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| member.export.to_string());
                    return Some(ImportTarget::CurrentModuleSymbol { symbol });
                }
            }

            if let ImportSource::LocalMember(m) = &mi.source {
                let local_path = m.local_path.clone()?;
                let span_table = &ast.span_table;
                let mspan = span_table.get(m.member.span);
                if byte_pos >= mspan.start() && byte_pos <= mspan.end() {
                    let symbol = m
                        .member
                        .alias
                        .as_ref()
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| m.member.export.to_string());
                    return Some(ImportTarget::LocalBundleSymbol { local_path, symbol });
                }
            }
        }
    }

    // Phase 2: bare word matching an import's effective name.
    let word = ast
        .word_at_byte(byte_pos, source)
        .or_else(|| source.word_at_byte_offset(byte_pos));

    if let Some(word) = word {
        for import in &ast.uses {
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
                        if member.flat_name().as_str() != word {
                            continue;
                        }
                        return match &b.local_path {
                            Some(lp) => Some(ImportTarget::LocalBundleSymbol {
                                local_path: lp.clone(),
                                symbol: member.export.to_string(),
                            }),
                            None => Some(ImportTarget::DepBundleMember {
                                dep_name: b.root.to_string(),
                                path_segments: b
                                    .path_segments
                                    .iter()
                                    .map(|s| s.to_string())
                                    .collect(),
                                symbol: member.export.to_string(),
                            }),
                        };
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

                if let ImportSource::LocalMember(m) = &mi.source {
                    let member_name = m.member.alias.unwrap_or(m.member.export).to_string();
                    if member_name == word {
                        let local_path = m.local_path.clone()?;
                        return Some(ImportTarget::LocalBundleSymbol {
                            local_path,
                            symbol: m.member.export.to_string(),
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

/// Resolve hover markdown for an import target at the cursor.
pub fn hover_for_import_target(
    file_path: &Path,
    target: ImportTarget,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<String> {
    match target {
        ImportTarget::DepRoot { dep_name } => resolve_dep_hover(file_path, &dep_name),
        ImportTarget::DepSymbol { dep_name, symbol }
        | ImportTarget::BodySymbol { dep_name, symbol } => {
            resolve_symbol_hover(file_path, &dep_name, &symbol, file_reader)
        }
        ImportTarget::DepBundleMember {
            dep_name,
            path_segments,
            symbol,
        } => resolve_dep_bundle_symbol_hover(
            file_path,
            &dep_name,
            &path_segments,
            &symbol,
            file_reader,
        ),
        ImportTarget::LocalBundleSymbol { local_path, symbol } => {
            resolve_local_symbol_hover(file_path, &local_path, &symbol, file_reader)
        }
        ImportTarget::CurrentModuleSymbol { symbol } => {
            resolve_current_module_hover(file_path, &symbol, file_reader)
        }
    }
}

/// Resolve definition byte range for an import target.
pub fn def_span_for_import_target(
    file_path: &Path,
    target: ImportTarget,
    byte_pos: usize,
    ast: &FileAst,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<std::ops::Range<usize>> {
    use ast::ImportSource;

    match target {
        ImportTarget::DepRoot { dep_name } => {
            resolve_dep_dir(file_path, &dep_name)?;
            for import in &ast.uses {
                for mi in &import.0 {
                    if let ImportSource::Package(mp) = &mi.source {
                        let span_table = &ast.span_table;
                        let span = span_table.get(mp.span_id());
                        if byte_pos >= span.start() && byte_pos <= span.end() {
                            return Some(span.start()..span.start());
                        }
                    }
                }
            }
            None
        }
        ImportTarget::DepSymbol { dep_name, symbol }
        | ImportTarget::BodySymbol { dep_name, symbol } => {
            resolve_symbol_def_span(file_path, &dep_name, &symbol, file_reader)
        }
        ImportTarget::DepBundleMember {
            dep_name,
            path_segments,
            symbol,
        } => resolve_dep_bundle_symbol_def_span(
            file_path,
            &dep_name,
            &path_segments,
            &symbol,
            file_reader,
        ),
        ImportTarget::LocalBundleSymbol { local_path, symbol } => {
            resolve_local_symbol_def_span(file_path, &local_path, &symbol, file_reader)
        }
        ImportTarget::CurrentModuleSymbol { symbol } => {
            resolve_current_module_def_span(file_path, &symbol, file_reader)
        }
    }
}

pub fn resolve_dep_hover(file_path: &Path, dep_name: &str) -> Option<String> {
    let dep_dir = resolve_dep_dir(file_path, dep_name)?;
    let dep_config = FlaskConfig::find_package_config(&dep_dir).map(|(cfg, _)| cfg)?;
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
    let def_loc =
        crate::symbol_location::dep_symbol_def_location(file_path, dep_name, symbol, file_reader)?;
    let parsed = file_reader(&def_loc.file)?;
    let typed = typecheck::transform::transform_file(parsed.output.ast, typecheck::FileId(0));
    let source = &parsed.source;
    let byte = def_loc.byte_range.start;
    let (line, character) = source.byte_offset_to_position(byte);
    typed.hover_at(source, line, character)
}

pub fn resolve_symbol_def_span(
    file_path: &Path,
    dep_name: &str,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<std::ops::Range<usize>> {
    crate::symbol_location::dep_symbol_def_location(file_path, dep_name, symbol, file_reader)
        .map(|loc| loc.byte_range)
}

pub fn resolve_dep_bundle_symbol_hover(
    file_path: &Path,
    dep_name: &str,
    path_segments: &[String],
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<String> {
    // TODO: re-implement hover for dep bundle symbols using
    //   typed = typecheck::transform_file(parsed.output.ast.clone(), typecheck::FileId(0));
    //   and typed.hover_at(...); `definition_span` was moved to resolve.
    let _ = (file_path, dep_name, path_segments, symbol, file_reader);
    None
}

pub fn resolve_dep_bundle_symbol_def_span(
    file_path: &Path,
    dep_name: &str,
    path_segments: &[String],
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<std::ops::Range<usize>> {
    use ast::LocalBundleImport;
    use ast::span::SpanId;
    use internment::Intern;

    let b = LocalBundleImport {
        root: Intern::new(dep_name.to_string()),
        path_segments: path_segments
            .iter()
            .map(|s| Intern::new(s.clone()))
            .collect(),
        members: Vec::new(),
        span: SpanId::INVALID,
        local_path: None,
    };
    crate::symbol_location::dep_bundle_member_def_location(file_path, &b, symbol, file_reader)
        .map(|loc| loc.byte_range)
}

pub fn resolve_local_symbol_hover(
    file_path: &Path,
    local_import_path: &Path,
    symbol: &str,
    _file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<String> {
    // TODO: re-implement hover for local bundle symbols using
    //   typed = typecheck::transform_file(ast.clone(), typecheck::FileId(0));
    //   and typed.hover_at(...); `definition_span` was moved to resolve.
    let _ = (file_path, local_import_path, symbol);
    None
}

pub fn resolve_local_symbol_def_span(
    file_path: &Path,
    local_import_path: &Path,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<std::ops::Range<usize>> {
    crate::symbol_location::local_bundle_def_location(
        file_path,
        local_import_path,
        symbol,
        file_reader,
    )
    .map(|loc| loc.byte_range)
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
    let def_loc = crate::symbol_location::current_module_package_def_location(
        file_path,
        symbol,
        file_reader,
    )?;
    let parsed = file_reader(&def_loc.file)?;
    let typed = typecheck::transform::transform_file(parsed.output.ast, typecheck::FileId(0));
    let source = &parsed.source;
    let byte = def_loc.byte_range.start;
    let (line, character) = source.byte_offset_to_position(byte);
    typed.hover_at(source, line, character)
}

pub fn resolve_current_module_def_span(
    file_path: &Path,
    symbol: &str,
    file_reader: &dyn Fn(&Path) -> Option<ParsedFile>,
) -> Option<std::ops::Range<usize>> {
    crate::symbol_location::current_module_package_def_location(file_path, symbol, file_reader)
        .map(|loc| loc.byte_range)
}

/// Resolve the definition span for a `use Symbol` import (current module).
#[cfg(test)]
mod tests {
    use super::part_index_in_dotted_path;

    #[test]
    fn part_index_root() {
        assert_eq!(part_index_in_dotted_path("core.true", 0), 0);
        assert_eq!(part_index_in_dotted_path("core.true", 3), 0);
    }

    #[test]
    fn part_index_first_segment() {
        assert_eq!(part_index_in_dotted_path("core.true", 5), 1);
        assert_eq!(part_index_in_dotted_path("core.true", 8), 1);
    }

    #[test]
    fn part_index_multi_segment() {
        assert_eq!(part_index_in_dotted_path("a.b.c", 0), 0);
        assert_eq!(part_index_in_dotted_path("a.b.c", 1), 0);
        assert_eq!(part_index_in_dotted_path("a.b.c", 2), 1);
        assert_eq!(part_index_in_dotted_path("a.b.c", 3), 1);
        assert_eq!(part_index_in_dotted_path("a.b.c", 4), 2);
        assert_eq!(part_index_in_dotted_path("a.b.c", 5), 2);
        assert_eq!(part_index_in_dotted_path("a.b.c", 6), 2);
    }

    #[test]
    fn part_index_edge_right_at_dot() {
        assert_eq!(part_index_in_dotted_path("core.true", 4), 0);
        assert_eq!(part_index_in_dotted_path("a.b.c", 2), 1);
        assert_eq!(part_index_in_dotted_path("a.b.c", 5), 2);
    }
}
