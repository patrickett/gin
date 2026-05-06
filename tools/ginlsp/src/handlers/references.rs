use crate::diagnostics::span_to_range;
use crate::Backend;
use database::file_parse_output;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use typeck::{find_references, get_word_at_position};

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

        let (source, file) = match self.documents.get(&uri.to_string()) {
            Some(state) => (state.source.clone(), state.file),
            None => return Ok(None),
        };

        let Some(word) = get_word_at_position(&source, position.line, position.character) else {
            return Ok(None);
        };

        // `file_parse_output` runs the parser; offload so a hang cannot pin
        // the async runtime.
        let uri_for_locs = uri.clone();
        let locations = self
            .run_blocking_request("references", move |this| {
                let snapshot = this.snapshot();
                let ast = file_parse_output(&snapshot.db, file).ast.clone();
                find_references(&ast, &word)
                    .into_iter()
                    .map(|span| Location {
                        uri: uri_for_locs.clone(),
                        range: span_to_range(span.start, span.end, &source),
                    })
                    .collect::<Vec<_>>()
            })
            .await;

        match locations {
            Some(locs) if !locs.is_empty() => Ok(Some(locs)),
            _ => Ok(None),
        }
    }
}
