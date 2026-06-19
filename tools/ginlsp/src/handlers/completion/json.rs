use crate::Backend;
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position, Url};

impl Backend {
    pub(crate) fn complete_flask_json(
        source: &str,
        position: Position,
        uri: &Url,
    ) -> Vec<CompletionItem> {
        if !Backend::is_flask_json_file(uri) {
            return vec![];
        }

        let line = source.lines().nth(position.line as usize).unwrap_or("");
        let before_cursor = &line[..(position.character as usize).min(line.len())];

        if before_cursor.contains("\"dependencies\"") {
            return Self::complete_dependency_keys();
        }

        Self::complete_top_level_fields()
    }

    // TODO: create something dynamic/better
    fn complete_top_level_fields() -> Vec<CompletionItem> {
        vec![
            Self::item("name", CompletionItemKind::FIELD, "Required – package name"),
            Self::item(
                "version",
                CompletionItemKind::FIELD,
                "Required – semver version",
            ),
            Self::item(
                "dependencies",
                CompletionItemKind::FIELD,
                "Optional – package dependencies",
            ),
            Self::item(
                "authors",
                CompletionItemKind::FIELD,
                "Optional – list of authors",
            ),
            Self::item(
                "description",
                CompletionItemKind::FIELD,
                "Optional – package description",
            ),
            Self::item(
                "license",
                CompletionItemKind::FIELD,
                "Optional – license(s)",
            ),
            Self::item(
                "repository",
                CompletionItemKind::FIELD,
                "Optional – repo URL",
            ),
            Self::item(
                "keywords",
                CompletionItemKind::FIELD,
                "Optional – package keywords",
            ),
            Self::item(
                "target",
                CompletionItemKind::FIELD,
                "Compile target triple (e.g. x86_64-unknown-linux-gnu) or \"library\"",
            ),
            Self::item(
                "exclude",
                CompletionItemKind::FIELD,
                "Optional – files to exclude",
            ),
            Self::item(
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
}
