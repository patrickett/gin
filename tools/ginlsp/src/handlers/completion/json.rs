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

    if before_cursor.contains("\"dependencies\"") {
        return complete_dependency_keys();
    }

    complete_top_level_fields()
}

// TODO: create something dynamic/better
fn complete_top_level_fields() -> Vec<CompletionItem> {
    vec![
        item("name", CompletionItemKind::FIELD, "Required – package name"),
        item(
            "version",
            CompletionItemKind::FIELD,
            "Required – semver version",
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

fn item(label: &str, kind: CompletionItemKind, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: Some(detail.to_string()),
        ..Default::default()
    }
}
