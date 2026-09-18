use crate::Backend;
use ast::source::SourceExt;
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

        let col = line_text.position_to_byte_offset(0, position.character)?;
        let before_cursor = &line_text[..col];

        if let Some((module, partial)) = Self::source_first_import_context(source, position) {
            return Some(Self::complete_bundle_members(
                file_uri, config, module, &partial, position,
            ));
        }
        if !trimmed.starts_with("use ") {
            return None;
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

    fn source_first_import_context(source: &str, position: Position) -> Option<(&str, String)> {
        let lines: Vec<_> = source.lines().collect();
        let line = *lines.get(position.line as usize)?;
        let end = line.position_to_byte_offset(0, position.character)?;
        let prefix = &line[..end];
        let mut header = position.line as usize;
        if !prefix.trim_start().starts_with("use ") {
            let indent = prefix.len() - prefix.trim_start().len();
            if indent == 0 {
                return None;
            }
            while header > 0 {
                header -= 1;
                let previous = lines[header];
                if previous.trim().is_empty() || previous.trim_start().starts_with("--") {
                    continue;
                }
                if previous.len() - previous.trim_start().len() < indent {
                    break;
                }
            }
        }
        let declaration = if header == position.line as usize {
            prefix
        } else {
            lines[header]
        };
        let rest = declaration.trim_start().strip_prefix("use ")?;
        let source_end = if rest.starts_with(['\'', '"']) {
            let quote = rest.chars().next()?;
            rest[1..].find(quote)? + 2
        } else {
            rest.find(char::is_whitespace).unwrap_or(rest.len())
        };
        let module = &rest[..source_end];
        let tail = &rest[source_end..];
        if module.is_empty()
            || module.contains([',', '('])
            || (!tail.is_empty() && !tail.starts_with(char::is_whitespace))
            || tail.trim_start().starts_with("as ")
            || (header == position.line as usize && tail.is_empty())
        {
            return None;
        }
        let mut partial = tail.trim_start().to_string();
        for previous in lines.iter().take(position.line as usize).skip(header + 1) {
            if previous.trim_start().starts_with("--") {
                continue;
            }
            partial.push('\n');
            partial.push_str(previous.trim());
        }
        if header != position.line as usize {
            partial.push('\n');
            partial.push_str(prefix.trim_start());
        }
        Some((module, partial))
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

        items
    }

    fn nested_folder_module_names(package_dir: &std::path::Path) -> Vec<String> {
        resolve::child_folder_module_names(package_dir)
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
        if let Some(quote) = trimmed.chars().next().filter(|c| matches!(c, '\'' | '"'))
            && let Some(quoted) = trimmed
                .strip_prefix(quote)
                .and_then(|s| s.strip_suffix(quote))
        {
            let pkg_root = resolve::find_package_root(&file_uri.to_file_path().ok()?)?;
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

        let parts: Vec<&str> = partial.split([',', '\n']).map(|s| s.trim()).collect();
        let already: Vec<String> = parts[..parts.len().saturating_sub(1)]
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.split_whitespace().next().unwrap_or_default().to_string())
            .collect();
        let current_partial = parts.last().unwrap_or(&"").trim();
        let edit_start_col = position
            .character
            .saturating_sub(current_partial.encode_utf16().count() as u32);

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
            let new_text = sym.clone();
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
#[path = "tests/path_tests.rs"]
mod tests;
