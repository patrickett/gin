use crate::Backend;
use ast::FileAst;

use ast::completions::{fn_call_at, signature_for_fn};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use ast::position_to_byte_offset;

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
            .to_string();
        let position = params.text_document_position_params.position;

        // Drop the DashMap ref before the spawn_blocking await: it is `!Send`
        // and would prevent the future from being scheduled. `file_parse_output`
        // runs the parser via Salsa and is the part that can hang on bad input.
        let (source, file_path) = match self.documents.get(&uri) {
            Some(state) => (state.source.clone(), state.file_path.clone()),
            None => return Ok(None),
        };

        let result = self
            .run_blocking_request("signature_help", move |this| {
                let snapshot = this.snapshot();
                let ast = snapshot.engine.parse_output(&file_path)?.ast.clone();
                build_signature_help(&source, &ast, position)
            })
            .await;

        Ok(result.flatten())
    }
}

fn build_signature_help(source: &str, ast: &FileAst, position: Position) -> Option<SignatureHelp> {
    let byte_pos = position_to_byte_offset(source, position.line, position.character)?;
    let fn_name = fn_call_at(ast, byte_pos)?;
    let info = signature_for_fn(ast, &fn_name)?;

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
