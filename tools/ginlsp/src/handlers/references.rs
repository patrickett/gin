use crate::Backend;
use crate::diagnostics::DiagnosticConverter;
use crate::handlers::{byte_position, request_target};

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

        let Some(target) = request_target(&params.text_document_position) else {
            return Ok(None);
        };

        let uri_for_locs = target.uri.clone();
        let locations = self
            .run_blocking_request("references", move |this| {
                let snapshot = this.snapshot();
                let doc = snapshot.cache.document_snapshot(&target.file_path)?;
                let converter =
                    DiagnosticConverter::new((*doc.source).clone(), (*doc.line_index).clone());
                let byte_pos = byte_position(&doc, target.position)? as u32;
                let refs = snapshot.cache.references_at(&target.file_path, byte_pos)?;
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
