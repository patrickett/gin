use crate::diagnostics::span_to_range;
use crate::Backend;
use lsp::{get_word_at_position, find_references};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_references(
        &self,
        params: ReferenceParams,
    ) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri.clone();
        let position = params.text_document_position.position;

        if let Some(state) = self.documents.get(&uri.to_string()) {
            if let Some(word) =
                get_word_at_position(&state.source, position.line, position.character)
            {
                let snapshot = self.snapshot();
                let ast = snapshot.parse(state.file);
                let locations: Vec<Location> = find_references(&ast, &word)
                    .into_iter()
                    .map(|span| Location {
                        uri: uri.clone(),
                        range: span_to_range(span.start, span.end, &state.source),
                    })
                    .collect();
                if !locations.is_empty() {
                    return Ok(Some(locations));
                }
            }
        }

        Ok(None)
    }
}
