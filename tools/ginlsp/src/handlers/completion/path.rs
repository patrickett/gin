use crate::Backend;
use flask::FlaskPathExt;
use resolve::GinPackageExt;
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position, Range, Url};

impl Backend {
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

        // Inside `use path.(partial` — bundle member list.
        if let Some(dot_paren_pos) = before_cursor.rfind(".(") {
            let module_path = &line_text[..dot_paren_pos].trim();
            if let Some(use_end) = module_path.rfind("use ") {
                let mod_name = module_path[use_end + 4..].trim();
                let partial = &before_cursor[dot_paren_pos + 2..];
                return Some(Self::complete_bundle_members(
                    file_uri, config, mod_name, partial, position,
                ));
            }
        }

        // After `use core.` or `use 'arch'.` — member import completions (`.Symbol`).
        if before_cursor.ends_with('.')
            && !before_cursor.contains(".(")
            && let Some(use_end) = before_cursor.rfind("use ")
        {
            let path_part = before_cursor[use_end + 4..].trim_end_matches('.').trim();
            if !path_part.is_empty() && !path_part.contains(',') {
                return Some(Self::complete_module_member_after_dot(
                    file_uri, config, path_part, position,
                ));
            }
        }

        if let Some(quote_pos) = before_cursor.rfind('\'') {
            let partial = &before_cursor[quote_pos + 1..];
            return Some(Self::complete_local_paths(
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
            let dep_names = Self::dependency_names(file_uri, config);
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
                items.extend(Self::complete_current_module_symbols(
                    file_uri,
                    position,
                    token_start,
                    token,
                ));
                return Some(items);
            }
        }

        // Non-quoted path: dependency root (`use dep`) or nested folder modules (`use dep.seg1.seg2`).
        Some(Self::complete_package_paths(
            file_uri, config, line_text, position,
        ))
    }

    fn dependency_names(file_uri: &Url, config: Option<&flask::FlaskConfigHandle>) -> Vec<String> {
        if let Some(handle) = config {
            let cfg = handle.read();
            return cfg
                .config
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

        match flask::FlaskConfig::find_package_config(file_dir).map(|(cfg, _)| cfg) {
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

        if flask::FlaskConfig::find_package_config(file_dir).is_none() {
            return vec![];
        }

        let mut items: Vec<CompletionItem> = Vec::new();

        for sym in file_dir.list_public_symbols() {
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
            return Self::complete_dependency_names(file_uri, config);
        }

        // If there's no dot, we're completing dependency root names.
        if !token.contains('.') {
            return Self::complete_dependency_names(file_uri, config);
        }

        let (dep_name, rest) = token.split_once('.').unwrap_or((token, ""));
        if dep_name.is_empty() {
            return vec![];
        }

        let Some(dep_dir) = Self::resolve_dep_dir(file_uri, config, dep_name) else {
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

        let Some(base_pkg_dir) = Self::walk_nested_package_dir(&dep_dir, &segments) else {
            return vec![];
        };

        let base_prefix = &token[..token.rfind('.').unwrap_or(token.len()) + 1];
        let mut items: Vec<CompletionItem> = Vec::new();

        // Add nested folder modules (sub-packages with flask.jsonc).
        for k in Self::nested_folder_module_names(&base_pkg_dir) {
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
        for sym in base_pkg_dir.list_public_symbols() {
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
            if p.is_dir() && Self::has_gin_files_in_tree(&p) {
                names.push(e.file_name().to_string_lossy().into_owned());
            }
        }
        names.sort();
        names
    }

    fn has_gin_files_in_tree(dir: &std::path::Path) -> bool {
        if !dir.list_package_gin_files().is_empty() {
            return true;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return false;
        };
        for e in rd.flatten() {
            if e.path().is_dir() && Self::has_gin_files_in_tree(&e.path()) {
                return true;
            }
        }
        false
    }

    fn walk_nested_package_dir(
        dep_dir: &std::path::Path,
        segments: &[&str],
    ) -> Option<std::path::PathBuf> {
        if segments.is_empty() {
            return Some(dep_dir.to_path_buf());
        }
        resolve::resolve_logical_module_dir(dep_dir, segments)
    }

    fn resolve_dep_dir(
        file_uri: &Url,
        config: Option<&flask::FlaskConfigHandle>,
        dep_name: &str,
    ) -> Option<std::path::PathBuf> {
        let (cfg, cfg_dir) = Self::load_config_and_dir(file_uri, config)?;
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
        flask::FlaskConfig::find_package_config(file_dir)
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

    /// Resolve a folder module directory for `core`, `core.io`, or `'arch/sub'`.
    fn resolve_module_dir_for_import_path(
        file_uri: &Url,
        config: Option<&flask::FlaskConfigHandle>,
        file_dir: &std::path::Path,
        mod_path: &str,
    ) -> Option<std::path::PathBuf> {
        let trimmed = mod_path.trim();
        if let Some(quoted) = trimmed
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
        {
            let pkg_root = resolve::find_package_root(file_dir)?;
            return resolve::normalize_local_import_path(
                file_dir,
                &pkg_root,
                std::path::Path::new(quoted),
            )
            .ok();
        }

        let (dep_name, rest) = match trimmed.split_once('.') {
            Some((d, r)) => (d, r),
            None => (trimmed, ""),
        };
        let dep_dir = Self::resolve_dep_dir(file_uri, config, dep_name)?;
        let segments: Vec<&str> = if rest.is_empty() {
            vec![]
        } else {
            rest.split('.').collect()
        };
        resolve::resolve_logical_module_dir(&dep_dir, &segments)
    }

    /// `use core.|` or `use 'arch'.|` — offer `.Symbol` member imports.
    fn complete_module_member_after_dot(
        file_uri: &Url,
        config: Option<&flask::FlaskConfigHandle>,
        mod_path: &str,
        position: Position,
    ) -> Vec<CompletionItem> {
        let file_path = match file_uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return vec![],
        };
        let file_dir = match file_path.parent() {
            Some(d) => d,
            None => return vec![],
        };
        let Some(mod_dir) =
            Self::resolve_module_dir_for_import_path(file_uri, config, file_dir, mod_path)
        else {
            return vec![];
        };

        let dot_col = position.character.saturating_sub(1);
        let text_edit_range = Range {
            start: Position {
                line: position.line,
                character: dot_col,
            },
            end: position,
        };

        let mut items = Vec::new();
        for sub in Self::nested_folder_module_names(&mod_dir) {
            items.push(CompletionItem {
                label: sub.clone(),
                kind: Some(CompletionItemKind::MODULE),
                detail: Some("folder module".to_string()),
                text_edit: Some(tower_lsp::lsp_types::CompletionTextEdit::Edit(
                    tower_lsp::lsp_types::TextEdit {
                        range: text_edit_range,
                        new_text: format!(".{sub}"),
                    },
                )),
                ..Default::default()
            });
        }
        for sym in mod_dir.list_public_symbols() {
            items.push(CompletionItem {
                label: sym.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(format!("{}.{}", mod_path.trim_matches('\''), sym)),
                text_edit: Some(tower_lsp::lsp_types::CompletionTextEdit::Edit(
                    tower_lsp::lsp_types::TextEdit {
                        range: text_edit_range,
                        new_text: format!(".{sym}"),
                    },
                )),
                ..Default::default()
            });
        }
        items
    }

    /// Inside `use path.(a, b|` — bundle members.
    fn complete_bundle_members(
        file_uri: &Url,
        config: Option<&flask::FlaskConfigHandle>,
        mod_name: &str,
        partial: &str,
        position: Position,
    ) -> Vec<CompletionItem> {
        let file_path = match file_uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return vec![],
        };
        let file_dir = match file_path.parent() {
            Some(d) => d,
            None => return vec![],
        };

        let Some(mod_dir) =
            Self::resolve_module_dir_for_import_path(file_uri, config, file_dir, mod_name)
        else {
            return vec![];
        };

        let parts: Vec<&str> = partial.split(',').map(|s| s.trim()).collect();
        let already: Vec<String> = parts[..parts.len().saturating_sub(1)]
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let current_partial = parts.last().unwrap_or(&"").trim();
        let after_last_comma = partial.rfind(',').map(|i| i + 1).unwrap_or(0);
        let edit_start_col = position
            .character
            .saturating_sub((partial.len() - after_last_comma) as u32);

        let text_edit_range = Range {
            start: Position {
                line: position.line,
                character: edit_start_col,
            },
            end: position,
        };

        let mut items = Vec::new();
        for sym in mod_dir.list_public_symbols() {
            if !sym.starts_with(current_partial) || already.iter().any(|s| s == &sym) {
                continue;
            }
            let new_text = if partial.contains(',') {
                format!(" {sym}")
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
                .config
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

        let config = match flask::FlaskConfig::find_package_config(file_dir).map(|(cfg, _)| cfg) {
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
        fs::create_dir_all(dir.join("dep/io")).unwrap();
        fs::create_dir_all(dir.join("dep/math")).unwrap();
        write_file(&dir.join("dep/io/socket.gin"), "connect := 1\n");
        write_file(&dir.join("dep/math/add.gin"), "add := 1\n");
        write_file(&dir.join("main.gin"), "use dep.\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let src = "use dep.\n";
        let pos = Position {
            line: 0,
            character: "use dep.".len() as u32,
        };
        let items = Backend::use_completions(src, pos, &uri, None).unwrap();
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
        fs::create_dir_all(dir.join("dep/a/b")).unwrap();
        fs::create_dir_all(dir.join("dep/a/c")).unwrap();
        write_file(&dir.join("dep/a/root.gin"), "a_root := 1\n");
        write_file(&dir.join("dep/a/b/one.gin"), "b_fn := 1\n");
        write_file(&dir.join("dep/a/c/two.gin"), "c_fn := 1\n");
        write_file(&dir.join("main.gin"), "use dep.a.\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let src = "use dep.a.\n";
        let pos = Position {
            line: 0,
            character: "use dep.a.".len() as u32,
        };
        let items = Backend::use_completions(src, pos, &uri, None).unwrap();
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
        fs::create_dir_all(dir.join("dep/io")).unwrap();
        fs::create_dir_all(dir.join("dep/math")).unwrap();
        write_file(&dir.join("dep/io/socket.gin"), "connect := 1\n");
        write_file(&dir.join("dep/math/add.gin"), "add := 1\n");
        write_file(&dir.join("main.gin"), "use dep.i\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let src = "use dep.i\n";
        let pos = Position {
            line: 0,
            character: "use dep.i".len() as u32,
        };
        let items = Backend::use_completions(src, pos, &uri, None).unwrap();
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
        let items = Backend::use_completions(src, pos, &uri, None).unwrap();
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        // Dependency name completion is currently unfiltered; ensure the deps appear.
        assert!(labels.contains(&"core"));
        assert!(labels.contains(&"dep"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn use_completion_package_member_import() {
        let dir = unique_temp_dir("member_import");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"{"name":"root","version":"0.0.0","authors":[],"dependencies":{"core":{"path":"core"}}}"#,
        );
        write_file(
            &dir.join("core/flask.jsonc"),
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        write_file(&dir.join("core/int.gin"), "Int is Unit\n");
        write_file(&dir.join("main.gin"), "use core.\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let src = "use core.\n";
        let pos = Position {
            line: 0,
            character: "use core.".len() as u32,
        };
        let items = Backend::use_completions(src, pos, &uri, None).unwrap();
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"Int"),
            "expected Int in completions after `use core.`, got {:?}",
            labels
        );

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
        let items = Backend::use_completions(src, pos, &uri, None).unwrap();
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
        let items = Backend::use_completions(src, pos, &uri, None).unwrap();
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"println"));
        assert!(labels.contains(&"print"));
        assert!(!labels.contains(&"Maybe"));

        let _ = fs::remove_dir_all(&dir);
    }
}
