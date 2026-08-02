use crate::Backend;
use crate::diagnostics::DiagnosticConverter;

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

        let file_path = match Backend::file_path_from_uri(&uri) {
            Some(p) => p,
            None => return Ok(None),
        };

        let uri_for_locs = uri.clone();
        let locations = self
            .run_blocking_request("references", move |this| {
                let snapshot = this.snapshot();
                let doc = snapshot.cache.document_snapshot(&file_path)?;
                let converter =
                    DiagnosticConverter::new((*doc.source).clone(), (*doc.line_index).clone());
                let byte_pos = doc.line_index.position_to_byte(
                    &doc.source,
                    position.line,
                    position.character,
                )? as u32;
                let refs = snapshot.cache.references_at(&file_path, byte_pos)?;
                Some(
                    refs.spans
                        .into_iter()
                        .map(|span| Location {
                            uri: uri_for_locs.clone(),
                            range: converter.span_to_range_indexed(span.start, span.end),
                        })
                        .collect::<Vec<Location>>(),
                )
            })
            .await;

        match locations {
            Some(Some(locs)) if !locs.is_empty() => Ok(Some(locs)),
            _ => Ok(None),
        }
    }
}
