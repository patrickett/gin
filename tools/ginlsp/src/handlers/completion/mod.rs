mod manifest;
mod path;

use std::path::PathBuf;

use crate::Backend;
use crate::handlers::{RequestTarget, byte_position, request_target};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use typecheck::completions::{CompletionCandidate, CompletionKind};

impl Backend {
    pub(crate) async fn handle_completion(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        if self.is_shutdown() {
            return Ok(None);
        }

        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;

        #[cfg(debug_assertions)]
        self.debug_log(format!("completion requested for URI: {}", uri))
            .await;

        // Manifest completions are handled separately via manifest_documents.
        if let Some(state) = self.manifest_documents.get(&uri) {
            let items = Backend::complete_manifest(
                &state.source,
                position,
                &params.text_document_position.text_document.uri,
            );
            #[cfg(debug_assertions)]
            self.debug_log(format!(
                "Returning {} completions for mod.flask",
                items.len()
            ))
            .await;
            return Ok(Some(CompletionResponse::Array(items)));
        }

        let Some(target) = request_target(&params.text_document_position) else {
            #[cfg(debug_assertions)]
            self.debug_log(format!("No file path found for URI: {}", uri))
                .await;
            return Ok(None);
        };

        let config = self.get_or_load_config(&target.uri);

        // Read source + line index from the engine for path completions
        // which need the current line text to parse `use` statements.
        let snapshot_st = self.snapshot();
        if let Some(doc) = snapshot_st.cache.document_snapshot(&target.file_path)
            && let Some(items) =
                Backend::use_completions(&doc.source, position, &target.uri, config.as_ref())
        {
            return Ok(Some(CompletionResponse::Array(items)));
        }

        let RequestTarget {
            file_path,
            position,
            ..
        } = target;
        let result = self
            .run_blocking_request("completion", move |this| {
                Self::compute_completions(&this, file_path, position)
            })
            .await;

        Ok(result.map(CompletionResponse::Array))
    }

    fn compute_completions(
        backend: &Backend,
        file_path: PathBuf,
        position: Position,
    ) -> Vec<CompletionItem> {
        let snapshot = backend.snapshot();
        let doc = match snapshot.cache.document_snapshot(&file_path) {
            Some(d) => d,
            None => return Vec::new(),
        };
        let Some(byte_pos) = byte_position(&doc, position) else {
            return Vec::new();
        };
        let candidates = snapshot.cache.completions_at(&file_path, byte_pos as u32);
        Self::candidates_to_lsp(candidates)
    }

    pub(crate) fn edit_range(position: Position, start_character: u32) -> Range {
        Range {
            start: Position::new(position.line, start_character),
            end: position,
        }
    }

    pub(crate) fn file_dir(uri: &Url) -> Option<PathBuf> {
        uri.to_file_path()
            .ok()?
            .parent()
            .map(std::path::Path::to_path_buf)
    }

    pub(crate) fn token_prefix(before_cursor: &str) -> (usize, &str) {
        let token_start = before_cursor
            .rfind(|c: char| c.is_whitespace() || c == ',')
            .map(|i| i + 1)
            .unwrap_or(0);
        (token_start, before_cursor[token_start..].trim())
    }

    fn completion_item(
        label: String,
        kind: CompletionItemKind,
        detail: String,
        edit: Option<TextEdit>,
    ) -> CompletionItem {
        CompletionItem {
            label,
            kind: Some(kind),
            detail: Some(detail),
            text_edit: edit.map(CompletionTextEdit::Edit),
            ..Default::default()
        }
    }

    fn dependency_item(name: String) -> CompletionItem {
        Self::completion_item(
            name,
            CompletionItemKind::MODULE,
            "dependency".to_string(),
            None,
        )
    }

    fn candidates_to_lsp(candidates: Vec<CompletionCandidate>) -> Vec<CompletionItem> {
        candidates
            .into_iter()
            .map(|c| {
                let kind = match c.kind {
                    CompletionKind::Function => CompletionItemKind::FUNCTION,
                    CompletionKind::Variable => CompletionItemKind::VARIABLE,
                    CompletionKind::Tag => CompletionItemKind::CLASS,
                    CompletionKind::Keyword => {
                        if c.label.starts_with('\'')
                            || c.label.chars().next().is_some_and(|ch| ch.is_ascii_digit())
                        {
                            CompletionItemKind::ENUM_MEMBER
                        } else {
                            CompletionItemKind::KEYWORD
                        }
                    }
                };
                let insert_text = if matches!(kind, CompletionItemKind::ENUM_MEMBER) {
                    Some(c.label.clone())
                } else {
                    None
                };
                let documentation = c.documentation.map(|doc| {
                    Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: doc,
                    })
                });
                CompletionItem {
                    label: c.label,
                    insert_text,
                    kind: Some(kind),
                    detail: c.detail,
                    documentation,
                    ..Default::default()
                }
            })
            .collect()
    }
}
