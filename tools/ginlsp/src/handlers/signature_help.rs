use crate::Backend;
use ast::FileAst;

use ast::source::SourceExt;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use typecheck::completions::{fn_call_at, signature_for_fn};

impl Backend {
    pub(crate) async fn handle_signature_help(
        &self,
        params: SignatureHelpParams,
    ) -> Result<Option<SignatureHelp>> {
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

        let result = self
            .run_blocking_request("signature_help", move |this| {
                let snapshot = this.snapshot();
                let (source, parse) = snapshot.engine.source_and_parse(&file_path)?;
                Self::build_signature_help(&source, &parse.ast, position)
            })
            .await;

        Ok(result.flatten())
    }

    fn build_signature_help(
        source: &str,
        ast: &FileAst,
        position: Position,
    ) -> Option<SignatureHelp> {
        let byte_pos = source.position_to_byte_offset(position.line, position.character)?;
        let (bind, _span_id) = fn_call_at(ast, byte_pos)?;
        let info = signature_for_fn(bind)?;

        let param_infos = info
            .params
            .iter()
            .map(|p| ParameterInformation {
                label: ParameterLabel::Simple(p.clone()),
                documentation: None,
            })
            .collect();

        let sig = SignatureInformation {
            label: info.label,
            documentation: info.documentation.map(|doc| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc,
                })
            }),
            parameters: Some(param_infos),
            active_parameter: None,
        };

        Some(SignatureHelp {
            signatures: vec![sig],
            active_signature: Some(0),
            active_parameter: None,
        })
    }
}
