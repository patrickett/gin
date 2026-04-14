use crate::Backend;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_formatting(
        &self,
        _params: DocumentFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        Ok(None)
    }
}
