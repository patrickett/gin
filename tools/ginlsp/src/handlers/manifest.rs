use crate::Backend;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range, Url};

impl Backend {
    pub(crate) async fn publish_manifest_diagnostics(&self, uri: Url, source: &str) {
        let Some(path) = Self::file_path_from_uri(&uri) else {
            return;
        };
        if path
            .file_name()
            .is_none_or(|name| name != flask::MANIFEST_NAME)
        {
            return;
        }
        let diagnostics = match flask::FlaskConfig::parse_manifest(source) {
            Ok(_) => Vec::new(),
            Err(error) => vec![Diagnostic {
                range: Range::new(
                    Position::new(error.line.saturating_sub(1), 0),
                    Position::new(error.line.saturating_sub(1), 1),
                ),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("gin".into()),
                message: error.message,
                ..Default::default()
            }],
        };
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}
