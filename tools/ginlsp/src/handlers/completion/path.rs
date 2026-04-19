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

    Some(complete_dependency_names(file_uri, config))
}

fn complete_local_paths(
    source: &str,
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
