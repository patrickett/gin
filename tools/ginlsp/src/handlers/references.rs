use crate::diagnostics::span_to_range;
use crate::Backend;
use database::file_parse_output;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use typeck::{find_references, position_to_byte_offset};

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

        let uri_for_locs = uri.clone();
        let locations = self
            .run_blocking_request("references", move |this| {
                let snapshot = this.snapshot();
                let ast = file_parse_output(&snapshot.db, file).ast.clone();
                let word = position_to_byte_offset(&source, position.line, position.character)
                    .and_then(|byte_pos| {
                        ast.word_at_byte(byte_pos, &source)
                            .or_else(|| typeck::word_at_byte_offset(&source, byte_pos))
                    })?;
                Some(
                    find_references(&ast, &word)
                        .into_iter()
                        .map(|span| Location {
                            uri: uri_for_locs.clone(),
                            range: span_to_range(span.start, span.end, &source),
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .await;

        match locations {
            Some(Some(locs)) if !locs.is_empty() => Ok(Some(locs)),
            _ => Ok(None),
        }
    }
}
