use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position, Range, Url};

pub(crate) fn use_completions(
    source: &str,
    position: Position,
    file_uri: &Url,
    config: Option<&flask::FlaskConfigHandle>,
) -> Option<Vec<CompletionItem>> {
    let line_text = source.lines().nth(position.line as usize)?;
    let trimmed = line_text.trim_start();

    if !trimmed.starts_with("use ") {
        return None;
    }

    let col = position.character as usize;
    let before_cursor = &line_text[..col.min(line_text.len())];

    // Check for `.()` bundle pattern — cursor is inside/after `.()`
    if let Some(dot_paren_pos) = before_cursor.rfind(".(") {
        let module_path = &line_text[..dot_paren_pos].trim();
        // Extract the module name: it's after `use ` and before `.()`
        if let Some(use_end) = module_path.rfind("use ") {
            let mod_name = module_path[use_end + 4..].trim();
            let partial = &before_cursor[dot_paren_pos + 2..];
            return Some(complete_bundle_members(
                file_uri, config, mod_name, partial, position, false,
            ));
        }
    }

    // Check for `'.'` after a module path (e.g. `use 'arch'.|`).
    // Offer public symbols from the module, wrapping selection in `.()`.
    // Match when the cursor is right after `.` and there's a quoted module path before it.
    let after_quote_dot = before_cursor
        .strip_suffix(".")
        .and_then(|s| s.rfind('\''))
        .is_some();
    if after_quote_dot {
        // Find the module name between `use ` and the closing quote before `.`
        if let Some(quote_start) = before_cursor.rfind('\'') {
            let before_quote = before_cursor[..quote_start].trim();
            if let Some(use_end) = before_quote.rfind("use ") {
                let mod_name = before_quote[use_end + 4..].trim();
                if !mod_name.is_empty() {
                    return Some(complete_bundle_members(
                        file_uri, config, mod_name, "", position, true,
                    ));
                }
            }
        }
    }

    if let Some(quote_pos) = before_cursor.rfind('\'') {
        let partial = &before_cursor[quote_pos + 1..];
        return Some(complete_local_paths(
            source, file_uri, position, quote_pos, partial,
        ));
    }

    // Determine the token currently being typed (after `use `).
    let token_start = before_cursor
        .rfind(|c: char| c.is_whitespace() || c == ',')
        .map(|i| i + 1)
        .unwrap_or(0);
    let token = before_cursor[token_start..].trim();

    // Bare identifier (no dot, no quote): offer current-module public symbols
    // alongside dependency names, unless the token matches a known dep exactly.
    if !token.contains('.') {
        let dep_names = dependency_names(file_uri, config);
        let is_known_dep = !token.is_empty() && dep_names.iter().any(|n| n == token);
        if !is_known_dep {
            let mut items: Vec<CompletionItem> = dep_names
                .into_iter()
                .map(|name| CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::MODULE),
                    detail: Some("dependency".to_string()),
                    ..Default::default()
                })
                .collect();
            items.extend(complete_current_module_symbols(
                file_uri,
                position,
                token_start,
                token,
            ));
            return Some(items);
        }
    }

    // Non-quoted path: dependency root (`use dep`) or nested folder modules (`use dep.seg1.seg2`).
    Some(complete_package_paths(
        file_uri, config, line_text, position,
    ))
}

fn dependency_names(file_uri: &Url, config: Option<&flask::FlaskConfigHandle>) -> Vec<String> {
    if let Some(handle) = config {
        let cfg = handle.read();
        return cfg
            .dependency_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect();
    }

    let file_path = match file_uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let file_dir = match file_path.parent() {
        Some(d) => d,
        None => return vec![],
    };

    match flask::FlaskConfig::from_directory(file_dir) {
        Some(cfg) => cfg
            .dependency_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect(),
        None => vec![],
    }
}

fn complete_current_module_symbols(
    file_uri: &Url,
    position: Position,
    token_start: usize,
    partial: &str,
) -> Vec<CompletionItem> {
    let file_path = match file_uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let file_dir = match file_path.parent() {
        Some(d) => d,
        None => return vec![],
    };

    let (_cfg, pkg_dir) = match flask::FlaskConfig::find_package_config(file_dir) {
        Some(result) => result,
        None => return vec![],
    };

    let mut items: Vec<CompletionItem> = Vec::new();

    for sym in resolve::list_public_symbols(&pkg_dir) {
        if !sym.starts_with(partial) {
            continue;
        }
        items.push(CompletionItem {
            label: sym.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("public symbol".to_string()),
            text_edit: Some(tower_lsp::lsp_types::CompletionTextEdit::Edit(
                tower_lsp::lsp_types::TextEdit {
                    range: Range {
                        start: Position {
                            line: position.line,
                            character: token_start as u32,
                        },
                        end: position,
                    },
                    new_text: sym,
                },
            )),
            ..Default::default()
        });
    }

    items
}

fn complete_package_paths(
    file_uri: &Url,
    config: Option<&flask::FlaskConfigHandle>,
    line_text: &str,
    position: Position,
) -> Vec<CompletionItem> {
    let col = position.character as usize;
    let before_cursor = &line_text[..col.min(line_text.len())];

    // Only complete the token currently being typed (after last space/comma).
    let token_start = before_cursor
        .rfind(|c: char| c.is_whitespace() || c == ',')
        .map(|i| i + 1)
        .unwrap_or(0);

    let token = before_cursor[token_start..].trim();
    if token.is_empty() {
        return complete_dependency_names(file_uri, config);
    }

    // If there's no dot, we're completing dependency root names.
    if !token.contains('.') {
        return complete_dependency_names(file_uri, config);
    }

    let (dep_name, rest) = token.split_once('.').unwrap_or((token, ""));
    if dep_name.is_empty() {
        return vec![];
    }

    let Some(dep_dir) = resolve_dep_dir(file_uri, config, dep_name) else {
        return vec![];
    };

    // Nested package names: immediate subdirectories of the dependency (or chained folder) that contain `flask.jsonc`.
    let mut segments: Vec<&str> = rest.split('.').collect();
    let ends_with_dot = token.ends_with('.');
    if ends_with_dot && segments.last().copied() == Some("") {
        segments.pop();
    }
    let partial = if ends_with_dot {
        ""
    } else {
        segments.pop().unwrap_or("")
    };

    let Some(base_pkg_dir) = walk_nested_package_dir(&dep_dir, &segments) else {
        return vec![];
    };

    let base_prefix = &token[..token.rfind('.').unwrap_or(token.len()) + 1];
    let mut items: Vec<CompletionItem> = Vec::new();

    // Add nested folder modules (sub-packages with flask.jsonc).
    for k in nested_folder_module_names(&base_pkg_dir) {
        if !k.starts_with(partial) {
            continue;
        }
        items.push(CompletionItem {
            label: k.clone(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some("nested package".to_string()),
            text_edit: Some(tower_lsp::lsp_types::CompletionTextEdit::Edit(
                tower_lsp::lsp_types::TextEdit {
                    range: Range {
                        start: Position {
                            line: position.line,
                            character: token_start as u32,
                        },
                        end: position,
                    },
                    new_text: format!("{base_prefix}{k}"),
                },
            )),
            ..Default::default()
        });
    }

    // Add public symbols (defs and tags) from .gin files in the package.
    for sym in resolve::list_public_symbols(&base_pkg_dir) {
        if !sym.starts_with(partial) {
            continue;
        }
        items.push(CompletionItem {
            label: sym.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some("public symbol".to_string()),
            text_edit: Some(tower_lsp::lsp_types::CompletionTextEdit::Edit(
                tower_lsp::lsp_types::TextEdit {
                    range: Range {
                        start: Position {
                            line: position.line,
                            character: token_start as u32,
                        },
                        end: position,
                    },
                    new_text: format!("{base_prefix}{sym}"),
                },
            )),
            ..Default::default()
        });
    }

    items
}

fn nested_folder_module_names(package_dir: &std::path::Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(rd) = std::fs::read_dir(package_dir) else {
        return names;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && p.join(flask::PACKAGE_CONFIG_NAME).is_file() {
            names.push(e.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    names
}

fn walk_nested_package_dir(
    dep_dir: &std::path::Path,
    segments: &[&str],
) -> Option<std::path::PathBuf> {
    if segments.is_empty() {
        return Some(dep_dir.to_path_buf());
    }
    match flask::resolve_nested_package_path(dep_dir, segments).ok()? {
        flask::NestedPackageTarget::FolderModule(dir) => Some(dir),
    }
}

fn resolve_dep_dir(
    file_uri: &Url,
    config: Option<&flask::FlaskConfigHandle>,
    dep_name: &str,
) -> Option<std::path::PathBuf> {
    let (cfg, cfg_dir) = load_config_and_dir(file_uri, config)?;
    let dep = cfg.dependencies().get(dep_name)?;
    match &dep.kind {
        flask::DependencyKind::Path { path } => Some(cfg_dir.join(path)),
        _ => None,
    }
}

fn load_config_and_dir(
    file_uri: &Url,
    config: Option<&flask::FlaskConfigHandle>,
) -> Option<(flask::FlaskConfig, std::path::PathBuf)> {
    if let Some(handle) = config {
        let cfg = handle.read();
        return Some((cfg.config.clone(), handle.source_dir()));
    }
    let file_path = file_uri.to_file_path().ok()?;
    let file_dir = file_path.parent()?;
    let cfg = flask::FlaskConfig::from_directory(file_dir)?;
    let cfg_dir = flask::FlaskConfigHandle::load(file_dir).ok()?.source_dir();
    Some((cfg, cfg_dir))
}

fn complete_local_paths(
    _source: &str,
    file_uri: &Url,
    position: Position,
    quote_pos: usize,
    partial: &str,
) -> Vec<CompletionItem> {
    let file_path = match file_uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let file_dir = match file_path.parent() {
        Some(d) => d,
        None => return vec![],
    };

    let search_dir = if partial.is_empty() {
        file_dir.to_path_buf()
    } else {
        let partial_path = std::path::Path::new(partial);
        let resolved = file_dir.join(partial_path);
        if partial.ends_with('/') {
            resolved
        } else {
            resolved.parent().unwrap_or(&resolved).to_path_buf()
        }
    };

    let prefix = if partial.contains('/') && !partial.ends_with('/') {
        let last_slash = partial.rfind('/').unwrap();
        &partial[..=last_slash]
    } else if partial.ends_with('/') {
        partial
    } else {
        ""
    };

    let mut items = Vec::new();
    let entries = match std::fs::read_dir(&search_dir) {
        Ok(e) => e,
        Err(_) => return items,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let insert_text = format!("{prefix}{name}'");
        let label = format!("{prefix}{name}");
        let kind = CompletionItemKind::FOLDER;

        // The text edit range replaces only the partial path after the quote
        let text_edit_range = Range {
            start: Position {
                line: position.line,
                character: (quote_pos + 1) as u32,
            },
            end: position,
        };

        items.push(CompletionItem {
            label,
            text_edit: Some(tower_lsp::lsp_types::CompletionTextEdit::Edit(
                tower_lsp::lsp_types::TextEdit {
                    range: text_edit_range,
                    new_text: insert_text,
                },
            )),
            kind: Some(kind),
            detail: Some("module".to_string()),
            ..Default::default()
        });
    }

    items
}

/// Complete public symbols from a module for `.()` bundle syntax.
/// `mod_name` is the module path before `.()` — can be a local path `'arch'`
/// or a dependency name `core`.
fn complete_bundle_members(
    file_uri: &Url,
    config: Option<&flask::FlaskConfigHandle>,
    mod_name: &str,
    partial: &str,
    position: Position,
    wrap_in_parens: bool,
) -> Vec<CompletionItem> {
    let file_path = match file_uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let file_dir = match file_path.parent() {
        Some(d) => d,
        None => return vec![],
    };

    // Resolve the module directory — allow directories without flask.jsonc
    let mod_dir = if mod_name.starts_with('\'') {
        let inner = mod_name.trim_matches('\'');
        file_dir.join(inner)
    } else {
        let dep_dir = resolve_dep_dir(file_uri, config, mod_name);
        match dep_dir {
            Some(d) => d,
            None => return vec![],
        }
    };

    if !mod_dir.is_dir() {
        return vec![];
    }

    // When inside `.()`, parse the partial to find already-listed members
    // and the current partial word being typed (after the last comma).
    let (already_listed, current_partial, edit_start_col) = if !wrap_in_parens {
        // Split by comma to find members already typed and the current partial
        let parts: Vec<&str> = partial.split(',').map(|s| s.trim()).collect();
        let (listed, cur) = if parts.is_empty() {
            (vec![], "")
        } else {
            // All but the last are already-listed members
            let already: Vec<String> = parts[..parts.len().saturating_sub(1)]
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            let current = parts.last().unwrap_or(&"").trim();
            (already, current)
        };
        // Edit start is from the end of the last comma (or after `(` if no comma)
        let after_last_comma = partial.rfind(',').map(|i| i + 1).unwrap_or(0);
        let edit_start = position
            .character
            .saturating_sub((partial.len() - after_last_comma) as u32);
        (listed, cur, edit_start)
    } else {
        (vec![], partial, position.character.saturating_sub(1))
    };

    let text_edit_range = if wrap_in_parens {
        // Replace from the `.` before cursor to cursor with `.(name)`
        Range {
            start: Position {
                line: position.line,
                character: edit_start_col,
            },
            end: position,
        }
    } else {
        // Replace the current partial word after the last comma / `(`
        Range {
            start: Position {
                line: position.line,
                character: edit_start_col,
            },
            end: position,
        }
    };

    let mut items: Vec<CompletionItem> = Vec::new();
    for sym in resolve::list_public_symbols(&mod_dir) {
        if !sym.starts_with(current_partial) {
            continue;
        }
        // Skip already-listed members
        if already_listed.iter().any(|s| s.as_str() == sym.as_str()) {
            continue;
        }
        let new_text = if wrap_in_parens {
            format!(".({})", sym)
        } else if partial.contains(',') {
            // Already has items — space before the name after the comma
            format!(" {}", sym)
        } else {
            sym.clone()
        };
        items.push(CompletionItem {
            label: sym.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(format!("{}.{}", mod_name.trim_matches('\''), sym)),
            text_edit: Some(tower_lsp::lsp_types::CompletionTextEdit::Edit(
                tower_lsp::lsp_types::TextEdit {
                    range: text_edit_range,
                    new_text,
                },
            )),
            ..Default::default()
        });
    }

    items
}

fn complete_dependency_names(
    file_uri: &Url,
    config: Option<&flask::FlaskConfigHandle>,
) -> Vec<CompletionItem> {
    if let Some(handle) = config {
        let cfg = handle.read();
        return cfg
            .dependency_names()
            .into_iter()
            .map(|name| CompletionItem {
                label: name.to_string(),
                kind: Some(CompletionItemKind::MODULE),
                detail: Some("dependency".to_string()),
                ..Default::default()
            })
            .collect();
    }

    let file_path = match file_uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let file_dir = match file_path.parent() {
        Some(d) => d,
        None => return vec![],
    };

    let config = match flask::FlaskConfig::from_directory(file_dir) {
        Some(c) => c,
        None => return vec![],
    };

    config
        .dependency_names()
        .into_iter()
        .map(|name| CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some("dependency".to_string()),
            ..Default::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        dir.push(format!("ginlsp_use_complete_{name}_{pid}_{nanos}"));
        dir
    }

    fn write_file(path: &std::path::Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn use_completion_dep_root_nested_packages() {
        let dir = unique_temp_dir("dep_root_exports");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "dep": { "path": "dep" } }
}
"#,
        );
        write_file(
            &dir.join("dep/flask.jsonc"),
            r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
        );
        write_file(
            &dir.join("dep/io/flask.jsonc"),
            r#"{"name":"io","version":"0.0.0","authors":[]}"#,
        );
        write_file(
            &dir.join("dep/math/flask.jsonc"),
            r#"{"name":"math","version":"0.0.0","authors":[]}"#,
        );
        write_file(&dir.join("main.gin"), "use dep.\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let src = "use dep.\n";
        let pos = Position {
            line: 0,
            character: "use dep.".len() as u32,
        };
        let items = use_completions(src, pos, &uri, None).unwrap();
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"io"));
        assert!(labels.contains(&"math"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn use_completion_chained_nested_packages_under_dep_a() {
        let dir = unique_temp_dir("dep_a_exports");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "dep": { "path": "dep" } }
}
"#,
        );
        write_file(
            &dir.join("dep/flask.jsonc"),
            r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
        );
        write_file(
            &dir.join("dep/a/flask.jsonc"),
            r#"{"name":"dep_a","version":"0.0.0","authors":[]}"#,
        );
        write_file(
            &dir.join("dep/a/b/flask.jsonc"),
            r#"{"name":"b","version":"0.0.0","authors":[]}"#,
        );
        write_file(
            &dir.join("dep/a/c/flask.jsonc"),
            r#"{"name":"c","version":"0.0.0","authors":[]}"#,
        );
        write_file(&dir.join("main.gin"), "use dep.a.\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let src = "use dep.a.\n";
        let pos = Position {
            line: 0,
            character: "use dep.a.".len() as u32,
        };
        let items = use_completions(src, pos, &uri, None).unwrap();
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"b"));
        assert!(labels.contains(&"c"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn use_completion_filters_nested_names_by_partial() {
        let dir = unique_temp_dir("dep_root_exports_partial");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "dep": { "path": "dep" } }
}
"#,
        );
        write_file(
            &dir.join("dep/flask.jsonc"),
            r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
        );
        write_file(
            &dir.join("dep/io/flask.jsonc"),
            r#"{"name":"io","version":"0.0.0","authors":[]}"#,
        );
        write_file(
            &dir.join("dep/math/flask.jsonc"),
            r#"{"name":"math","version":"0.0.0","authors":[]}"#,
        );
        write_file(&dir.join("main.gin"), "use dep.i\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let src = "use dep.i\n";
        let pos = Position {
            line: 0,
            character: "use dep.i".len() as u32,
        };
        let items = use_completions(src, pos, &uri, None).unwrap();
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"io"));
        assert!(!labels.contains(&"math"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn use_completion_without_dot_completes_dependency_names() {
        let dir = unique_temp_dir("dep_names");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "core": { "path": "core" }, "dep": { "path": "dep" } }
}
"#,
        );
        write_file(&dir.join("main.gin"), "use c\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let src = "use c\n";
        let pos = Position {
            line: 0,
            character: "use c".len() as u32,
        };
        let items = use_completions(src, pos, &uri, None).unwrap();
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        // Dependency name completion is currently unfiltered; ensure the deps appear.
        assert!(labels.contains(&"core"));
        assert!(labels.contains(&"dep"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn use_completion_shows_public_symbols() {
        let dir = unique_temp_dir("public_syms");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "dep": { "path": "dep" } }
}
"#,
        );
        write_file(
            &dir.join("dep/flask.jsonc"),
            r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
        );
        write_file(
            &dir.join("dep/io.gin"),
            "println(s Str): s\nprint(s Str): s\n",
        );
        write_file(&dir.join("dep/maybe.gin"), "Maybe is Some(value) or None\n");
        write_file(&dir.join("main.gin"), "use dep.\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let src = "use dep.\n";
        let pos = Position {
            line: 0,
            character: "use dep.".len() as u32,
        };
        let items = use_completions(src, pos, &uri, None).unwrap();
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"println"),
            "println not in labels: {:?}",
            labels
        );
        assert!(
            labels.contains(&"print"),
            "print not in labels: {:?}",
            labels
        );
        assert!(
            labels.contains(&"Maybe"),
            "Maybe not in labels: {:?}",
            labels
        );
        for item in &items {
            if item.label == "println" || item.label == "print" || item.label == "Maybe" {
                assert_eq!(
                    item.kind,
                    Some(CompletionItemKind::FUNCTION),
                    "expected FUNCTION kind for {}",
                    item.label
                );
            }
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn use_completion_filters_public_symbols_by_partial() {
        let dir = unique_temp_dir("public_syms_partial");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "dep": { "path": "dep" } }
}
"#,
        );
        write_file(
            &dir.join("dep/flask.jsonc"),
            r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
        );
        write_file(
            &dir.join("dep/io.gin"),
            "println(s Str): s\nprint(s Str): s\n",
        );
        write_file(&dir.join("dep/maybe.gin"), "Maybe is Some(value) or None\n");
        write_file(&dir.join("main.gin"), "use dep.p\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let src = "use dep.p\n";
        let pos = Position {
            line: 0,
            character: "use dep.p".len() as u32,
        };
        let items = use_completions(src, pos, &uri, None).unwrap();
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"println"));
        assert!(labels.contains(&"print"));
        assert!(!labels.contains(&"Maybe"));

        let _ = fs::remove_dir_all(&dir);
    }
}
