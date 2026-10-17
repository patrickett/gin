use crate::Backend;
use crate::handlers::{byte_position, byte_range_to_range, request_target};
use ast::source::SourceExt;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        if self.is_shutdown() {
            return Ok(None);
        }

        let Some(target) = request_target(&params.text_document_position_params) else {
            return Ok(None);
        };

        // Read source + line index from the engine (single source of truth).
        let snapshot_st = self.snapshot();
        let doc = match snapshot_st.cache.document_snapshot(&target.file_path) {
            Some(d) => d,
            None => return Ok(None),
        };

        let Some(byte_pos) = byte_position(&doc, target.position) else {
            return Ok(None);
        };

        if doc.source.is_comment_at(byte_pos) {
            return Ok(None);
        }

        if let Some(ch) = doc.source.as_bytes().get(byte_pos).map(|&b| b as char)
            && matches!(ch, '(' | ')' | '[' | ']')
        {
            return Ok(None);
        }

        let Some(byte_pos_u32) = u32::try_from(byte_pos).ok() else {
            return Ok(None);
        };

        let content = self
            .run_blocking_request("hover", move |this| {
                let snapshot = this.snapshot();
                snapshot.cache.hover(&target.file_path, byte_pos_u32)
            })
            .await
            .flatten();

        Ok(content.map(|content| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content.markdown,
            }),
            range: content
                .byte_range
                .map(|range| byte_range_to_range(range.start, range.end, &doc.source)),
        }))
    }
}
