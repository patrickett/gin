use crate::Backend;
use ast::source::SourceExt;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        if self.is_shutdown() {
            return Ok(None);
        }

        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        let position = params.text_document_position_params.position;

        let file_path = match Backend::file_path_from_uri(&uri) {
            Some(p) => p,
            None => return Ok(None),
        };

        // Read source + line index from the engine (single source of truth).
        let snapshot_st = self.snapshot();
        let doc = match snapshot_st.cache.document_snapshot(&file_path) {
            Some(d) => d,
            None => return Ok(None),
        };

        let Some(byte_pos) =
            doc.line_index
                .position_to_byte(&doc.source, position.line, position.character)
        else {
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
                snapshot.cache.hover(&file_path, byte_pos_u32)
            })
            .await
            .flatten();

        Ok(content.map(|content| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content.markdown,
            }),
            range: content.byte_range.map(|range| {
                let (s_line, s_char) = doc.line_index.byte_to_position(&doc.source, range.start);
                let (e_line, e_char) = doc.line_index.byte_to_position(&doc.source, range.end);
                Range {
                    start: Position {
                        line: s_line,
                        character: s_char,
                    },
                    end: Position {
                        line: e_line,
                        character: e_char,
                    },
                }
            }),
        }))
    }
}
