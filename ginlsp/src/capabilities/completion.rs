use ginc::{typeck::Ty, CompletionKind, FileAst};
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Documentation, MarkupContent, MarkupKind, Position, Url,
};

pub fn use_completions(
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
        return Some(complete_local_paths(file_uri, partial));
    }

    Some(complete_dependency_names(file_uri, config))
}

fn complete_local_paths(file_uri: &Url, partial: &str) -> Vec<CompletionItem> {
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

        let label = if is_dir {
            format!("{prefix}{name}/")
        } else {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            format!("{prefix}{stem}")
        };

        let kind = if is_dir {
            CompletionItemKind::FOLDER
        } else {
            CompletionItemKind::FILE
        };

        items.push(CompletionItem {
            label,
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
    // Use cached config if available
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

    // Fallback: load fresh (for when config wasn't cached yet)
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

pub fn build_completions(ast: &FileAst) -> Vec<CompletionItem> {
    ginc::completions_for_ast(ast)
        .into_iter()
        .map(|c| {
            let kind = match c.kind {
                CompletionKind::Function => CompletionItemKind::FUNCTION,
                CompletionKind::Variable => CompletionItemKind::VARIABLE,
                CompletionKind::Tag => CompletionItemKind::CLASS,
                CompletionKind::Keyword => CompletionItemKind::KEYWORD,
            };
            let documentation = c.documentation.map(|doc| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc,
                })
            });
            CompletionItem {
                label: c.label,
                kind: Some(kind),
                detail: c.detail,
                documentation,
                ..Default::default()
            }
        })
        .collect()
}

/// Build completion items for a dot expression from a resolved union type.
///
/// Called after the compiler resolves what type is before the dot.
/// Returns an empty vec for non-union types.
pub fn dot_completions(ty: Ty) -> Vec<CompletionItem> {
    let Ty::Union { name, variants } = ty else {
        return vec![];
    };
    let qualifier = name.as_str().to_string();
    variants
        .iter()
        .map(|(variant_name, fields)| {
            let label = if fields.is_empty() {
                variant_name.to_string()
            } else {
                let names: Vec<String> = fields.iter().map(|(n, _)| n.to_string()).collect();
                format!("{}({})", variant_name, names.join(", "))
            };
            let detail = format!("{}.{}", qualifier, label);
            CompletionItem {
                label: label.clone(),
                insert_text: Some(label),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                detail: Some(detail),
                ..Default::default()
            }
        })
        .collect()
}
