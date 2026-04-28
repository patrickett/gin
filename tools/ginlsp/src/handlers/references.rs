use crate::diagnostics::span_to_range;
use crate::Backend;
use database::file_parse_output;
use typeck::{find_references, get_word_at_position};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_references(
        &self,
        params: ReferenceParams,
    ) -> Result<Option<Vec<Location>>> {
        if self.is_shutdown() {
            return Ok(None);
        }

        let uri = params.text_document_position.text_document.uri.clone();
        let position = params.text_document_position.position;

        if let Some(state) = self.documents.get(&uri.to_string()) {
            if let Some(word) =
                get_word_at_position(&state.source, position.line, position.character)
            {
                let snapshot = self.snapshot();
                let ast = file_parse_output(&snapshot.db, state.file).ast.clone();
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
