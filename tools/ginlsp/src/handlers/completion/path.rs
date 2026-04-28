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

    if let Some(quote_pos) = before_cursor.rfind('\'') {
        let partial = &before_cursor[quote_pos + 1..];
        return Some(complete_local_paths(
            source,
            file_uri,
            position,
            quote_pos,
            partial,
        ));
    }

    // Non-quoted path: either dependency name completion (`use dep...`) or export-key completion
    // (`use dep.export` / `use dep.a.b`).
    Some(complete_package_paths(file_uri, config, line_text, position))
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

    // Determine which module's exports we are completing:
    // - `dep.` completes exports from dep root
    // - `dep.a.` completes exports from folder-module at dep.exports[a]
    // - `dep.a.b` completes exports at dep.exports[a], using `b` as partial
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

    // Walk intermediate segments, each must resolve to a folder-module.
    let Some(dep_cfg) = flask::FlaskConfig::from_directory(&dep_dir) else {
        return vec![];
    };
    let Some(exports_cfg) = walk_exports_chain(&dep_dir, &dep_cfg, &segments) else {
        return vec![];
    };

    let base_prefix = &token[..token.rfind('.').unwrap_or(token.len()) + 1];
    exports_cfg
        .exports()
        .keys()
        .filter(|k| k.starts_with(partial))
        .map(|k| CompletionItem {
            label: k.to_string(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some("export".to_string()),
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
        })
        .collect()
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

fn walk_exports_chain(
    dep_dir: &std::path::Path,
    dep_cfg: &flask::FlaskConfig,
    segments: &[&str],
) -> Option<flask::FlaskConfig> {
    if segments.is_empty() {
        return Some(dep_cfg.clone());
    }

    match flask::resolve_chained_exports(dep_dir, segments).ok()? {
        flask::ExportTarget::FolderModule(dir) => flask::FlaskConfig::from_directory(&dir),
        flask::ExportTarget::File(_) => None,
    }
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
        let is_dir = path.is_dir();
        let is_gin = path.extension().is_some_and(|e| e == "gin");

        if !is_dir && !is_gin {
            continue;
        }

        let insert_text = if is_dir {
            format!("{prefix}{name}/")
        } else {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            format!("{prefix}{stem}")
        };

        let label = insert_text.clone();

        let kind = if is_dir {
            CompletionItemKind::FOLDER
        } else {
            CompletionItemKind::FILE
        };

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
            detail: Some(if is_dir {
                "directory".to_string()
            } else {
                "gin module".to_string()
            }),
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
    fn use_completion_dep_root_exports() {
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
  "authors": [],
  "exports": { "io": { "path": "io.gin" }, "math": { "path": "math.gin" } }
}
"#,
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
    fn use_completion_chained_exports_dep_a_dot_completes_exports_in_a() {
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
  "authors": [],
  "exports": { "a": { "path": "a" } }
}
"#,
        );
        write_file(
            &dir.join("dep/a/flask.jsonc"),
            r#"
{
  "name": "dep_a",
  "version": "0.0.0",
  "authors": [],
  "exports": { "b": { "path": "b.gin" }, "c": { "path": "c.gin" } }
}
"#,
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
    fn use_completion_filters_export_keys_by_partial() {
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
  "authors": [],
  "exports": { "io": { "path": "io.gin" }, "math": { "path": "math.gin" } }
}
"#,
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
}
