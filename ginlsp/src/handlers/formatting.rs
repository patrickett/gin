use crate::Backend;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri.to_string();

        if let Some(state) = self.documents.get(&uri) {
            let formatted = ginfmt::format(&state.source);

            if formatted == *state.source {
                return Ok(None);
            }

            let full_range = Range {
                start: Position { line: 0, character: 0 },
                end: Position {
                    line: state.source.lines().count() as u32,
                    character: state.source.lines().last().map(|l| l.len() as u32).unwrap_or(0),
                },
            };

            Ok(Some(vec![TextEdit { range: full_range, new_text: formatted }]))
        } else {
            Ok(None)
        }
    }
}
