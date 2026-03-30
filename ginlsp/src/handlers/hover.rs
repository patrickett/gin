use crate::Backend;
use ginc::{get_char_at_position, get_number_at_position, is_in_comment, position_to_byte_offset};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_hover(
        &self,
        params: HoverParams,
    ) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri.clone();
        let position = params.text_document_position_params.position;

        if let Some(state) = self.documents.get(&uri.to_string()) {
            if is_in_comment(&state.source, position.line, position.character) {
                return Ok(None);
            }

            if let Some('(' | ')' | '[' | ']') =
                get_char_at_position(&state.source, position.line, position.character)
            {
                return Ok(None);
            }

            if let Some(num) =
                get_number_at_position(&state.source, position.line, position.character)
            {
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!("```gin\n{num}\n```"),
                    }),
                    range: None,
                }));
            }

            let snapshot = self.snapshot();
            if let Some(byte_pos) =
                position_to_byte_offset(&state.source, position.line, position.character)
            {
                if let Some(value) = snapshot.hover_at(state.file, byte_pos) {
                    return Ok(Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value,
                        }),
                        range: None,
                    }));
                }
            }
        }

        Ok(None)
    }
}
