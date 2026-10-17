use crate::Backend;
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, Position, Url};

impl Backend {
    pub(crate) fn complete_manifest(
        source: &str,
        position: Position,
        uri: &Url,
    ) -> Vec<CompletionItem> {
        if !Backend::is_manifest_file(uri) {
            return vec![];
        }
        let dependency_section = source
            .lines()
            .take(position.line as usize + 1)
            .filter(|line| line.trim().starts_with('#'))
            .last()
            .is_some_and(|line| line.trim() == "#dependencies");
        let labels: &[&str] = if dependency_section {
            &["#settings"]
        } else {
            &[
                "module: ",
                "version: ",
                "target: ",
                "description: ",
                "authors: ",
                "repository: ",
                "keywords: ",
                "license: ",
                "bugs: ",
                "funding: ",
                "#settings",
                "#dependencies",
            ]
        };
        labels
            .iter()
            .map(|label| Self::item(label, CompletionItemKind::FIELD, "mod.flask entry"))
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completions_only_handle_mod_flask_documents() {
        let source = "#settings\nmodule: 'app'\nversion: 1.0.0\n";
        let uri = Url::parse("file:///workspace/mod.flask").unwrap();
        let completions = Backend::complete_manifest(source, Position::new(1, 0), &uri);
        assert!(completions.iter().any(|item| item.label == "target: "));
        for filename in ["flask.json", "flask.jsonc", "other.flask"] {
            let uri = Url::parse(&format!("file:///workspace/{filename}")).unwrap();
            assert!(!Backend::should_handle_file(&uri));
            assert!(Backend::complete_manifest(source, Position::new(1, 0), &uri).is_empty());
        }
    }
}
