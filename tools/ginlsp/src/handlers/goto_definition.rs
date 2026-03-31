use crate::diagnostics::span_to_range;
use crate::Backend;
use lsp::{get_word_at_position, find_definition_span};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri.clone();
        let position = params.text_document_position_params.position;

        if let Some(state) = self.documents.get(&uri.to_string()) {
            let snapshot = self.snapshot();
            let ast = snapshot.parse(state.file);

            if let Some(word) =
                get_word_at_position(&state.source, position.line, position.character)
            {
                let range = find_definition_span(&ast, &word)
                    .map(|span| span_to_range(span.start, span.end, &state.source))
                    .unwrap_or_default();
                if range != Range::default() {
                    return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri,
                        range,
                    })));
                }
            }
        }

        Ok(None)
    }
}
