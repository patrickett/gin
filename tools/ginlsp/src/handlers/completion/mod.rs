pub(crate) mod json;
mod path;

use std::path::PathBuf;

use crate::Backend;
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

        let doc_uri = params.text_document_position.text_document.uri.clone();
        let uri = doc_uri.to_string();
        let position = params.text_document_position.position;

        #[cfg(debug_assertions)]
        self.client
            .log_message(
                MessageType::INFO,
                format!("completion requested for URI: {}", uri),
            )
            .await;

        // flask.jsonc completions are handled separately via json_documents.
        if let Some(state) = self.json_documents.get(&uri) {
            let items = Backend::complete_flask_json(&state.source, position, &doc_uri);
            #[cfg(debug_assertions)]
            self.client
                .log_message(
                    MessageType::INFO,
                    format!("Returning {} completions for flask.jsonc", items.len()),
                )
                .await;
            return Ok(Some(CompletionResponse::Array(items)));
        }

        let file_path = match Backend::file_path_from_uri(&doc_uri) {
            Some(p) => p,
            None => {
                #[cfg(debug_assertions)]
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!("No file path found for URI: {}", uri),
                    )
                    .await;
                return Ok(None);
            }
        };

        let config = self.get_or_load_config(&doc_uri);

        // Read source + line index from the engine for path completions
        // which need the current line text to parse `use` statements.
        let snapshot_st = self.snapshot();
        if let Some(doc) = snapshot_st.cache.document_snapshot(&file_path)
            && let Some(items) =
                Backend::use_completions(&doc.source, position, &doc_uri, config.as_ref())
        {
            return Ok(Some(CompletionResponse::Array(items)));
        }

        let file_path_clone = file_path.clone();
        let result = self
            .run_blocking_request("completion", move |this| {
                Self::compute_completions(&this, file_path_clone, position)
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
        let byte_pos =
            match doc
                .line_index
                .position_to_byte(&doc.source, position.line, position.character)
            {
                Some(b) => b as u32,
                None => return Vec::new(),
            };
        let candidates = snapshot.cache.completions_at(&file_path, byte_pos);
        Self::candidates_to_lsp(candidates)
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
