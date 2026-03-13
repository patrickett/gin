use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position, Url};

/// Check if this is a flask.json file
pub fn is_flask_json_file(uri: &Url) -> bool {
    uri.to_file_path()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy() == "flask.json"))
        .unwrap_or(false)
}

/// Check if this is a .gin file
pub fn is_gin_file(uri: &Url) -> bool {
    uri.to_file_path()
        .ok()
        .and_then(|p| p.extension().map(|e| e.to_string_lossy() == "gin"))
        .unwrap_or(false)
}

/// Check if this file should be handled by the language server
pub fn should_handle_file(uri: &Url) -> bool {
    is_gin_file(uri) || is_flask_json_file(uri)
}

/// Main entry point for flask.json completion
pub fn complete_flask_json(source: &str, position: Position, uri: &Url) -> Vec<CompletionItem> {
    if !is_flask_json_file(uri) {
        return vec![];
    }

    // Simple heuristic: find what field we're completing
    let line = source.lines().nth(position.line as usize).unwrap_or("");
    let before_cursor = &line[..(position.character as usize).min(line.len())];

    // Check if we're completing a field value (after "entry": ")
    if before_cursor.contains("\"entry\"") && !before_cursor.contains("\"entry\": \"") {
        // Cursor is before the colon, complete the field name
        return complete_top_level_fields();
    }

    // Check if we're after "entry": " - complete file paths
    if before_cursor.contains("\"entry\": \"") || before_cursor.ends_with("\"entry\":") {
        return complete_entry_values(uri);
    }

    // Check if we're inside dependencies object
    if before_cursor.contains("\"dependencies\"") {
        return complete_dependency_keys();
    }

    // Default: complete top-level fields
    complete_top_level_fields()
}

// TODO: create something dynamic/better
// also be able to predict values, like for entry we can probably list files and/or guess files
fn complete_top_level_fields() -> Vec<CompletionItem> {
    vec![
        item("name", CompletionItemKind::FIELD, "Required – package name"),
        item(
            "version",
            CompletionItemKind::FIELD,
            "Required – semver version",
        ),
        item(
            "entry",
            CompletionItemKind::FIELD,
            "Optional – main .gin file",
        ),
        item(
            "dependencies",
            CompletionItemKind::FIELD,
            "Optional – package dependencies",
        ),
        item(
            "authors",
            CompletionItemKind::FIELD,
            "Optional – list of authors",
        ),
        item(
            "description",
            CompletionItemKind::FIELD,
            "Optional – package description",
        ),
        item(
            "license",
            CompletionItemKind::FIELD,
            "Optional – license(s)",
        ),
        item(
            "repository",
            CompletionItemKind::FIELD,
            "Optional – repo URL",
        ),
        item(
            "keywords",
            CompletionItemKind::FIELD,
            "Optional – package keywords",
        ),
        item(
            "targets",
            CompletionItemKind::FIELD,
            "Optional – build targets",
        ),
        item(
            "exclude",
            CompletionItemKind::FIELD,
            "Optional – files to exclude",
        ),
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
    // Find .gin files in the project
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
