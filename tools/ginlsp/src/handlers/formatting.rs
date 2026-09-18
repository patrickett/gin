use crate::Backend;
use ast::source::SourceExt;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        if !Self::is_gin_file(&params.text_document.uri) {
            return Ok(None);
        }

        let Some(path) = Self::file_path_from_uri(&params.text_document.uri) else {
            return Ok(None);
        };
        let source = {
            let host = self.lock_host();
            host.cache
                .source_and_parse(&path)
                .and_then(|(source, parse)| {
                    if parse.symptoms.is_empty() {
                        Some(source)
                    } else {
                        None
                    }
                })
        };
        let Some(source) = source else {
            return Ok(None);
        };

        let formatted = ginfmt::format(&source);
        if formatted == *source {
            return Ok(Some(Vec::new()));
        }

        Ok(Some(vec![TextEdit::new(
            Range::new(Position::new(0, 0), {
                let (line, character) = source.byte_offset_to_position(source.len());
                Position::new(line, character)
            }),
            formatted,
        )]))
    }
}
