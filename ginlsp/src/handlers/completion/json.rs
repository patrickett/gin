use crate::handlers::is_flask_json_file;
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position, Url};

pub(crate) fn complete_flask_json(
    source: &str,
    position: Position,
    uri: &Url,
) -> Vec<CompletionItem> {
    if !is_flask_json_file(uri) {
        return vec![];
    }

    let line = source.lines().nth(position.line as usize).unwrap_or("");
    let before_cursor = &line[..(position.character as usize).min(line.len())];

    if before_cursor.contains("\"entry\"") && !before_cursor.contains("\"entry\": \"") {
        return complete_top_level_fields();
    }

    if before_cursor.contains("\"entry\": \"") || before_cursor.ends_with("\"entry\":") {
        return complete_entry_values(uri);
    }

    if before_cursor.contains("\"dependencies\"") {
        return complete_dependency_keys();
    }

    complete_top_level_fields()
}

// TODO: create something dynamic/better
// also be able to predict values, like for entry we can probably list files and/or guess files
fn complete_top_level_fields() -> Vec<CompletionItem> {
    vec![
        item("name", CompletionItemKind::FIELD, "Required – package name"),
        item("version", CompletionItemKind::FIELD, "Required – semver version"),
        item("entry", CompletionItemKind::FIELD, "Optional – main .gin file"),
        item("dependencies", CompletionItemKind::FIELD, "Optional – package dependencies"),
        item("authors", CompletionItemKind::FIELD, "Optional – list of authors"),
        item("description", CompletionItemKind::FIELD, "Optional – package description"),
        item("license", CompletionItemKind::FIELD, "Optional – license(s)"),
        item("repository", CompletionItemKind::FIELD, "Optional – repo URL"),
        item("keywords", CompletionItemKind::FIELD, "Optional – package keywords"),
        item("targets", CompletionItemKind::FIELD, "Optional – build targets"),
        item("exclude", CompletionItemKind::FIELD, "Optional – files to exclude"),
        item(
            "interface_hash",
            CompletionItemKind::FIELD,
            "Optional – hash for incremental compilation",
        ),
    ]
}

fn complete_dependency_keys() -> Vec<CompletionItem> {
    // Future: load from package registry
    vec![]
}

fn complete_entry_values(uri: &Url) -> Vec<CompletionItem> {
    let file_path = match uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let project_dir = match file_path.parent() {
        Some(d) => d,
        None => return vec![],
    };

    let mut items = vec![CompletionItem {
        label: "main.gin".to_string(),
        kind: Some(CompletionItemKind::FILE),
        detail: Some("default entry point".to_string()),
        ..Default::default()
    }];

    if let Ok(entries) = std::fs::read_dir(project_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gin") {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                items.push(CompletionItem {
                    label: name.to_string(),
                    kind: Some(CompletionItemKind::FILE),
                    ..Default::default()
                });
            }
        }
    }

    items
}

fn item(label: &str, kind: CompletionItemKind, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: Some(detail.to_string()),
        ..Default::default()
    }
}
