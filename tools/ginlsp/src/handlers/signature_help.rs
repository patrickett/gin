use crate::Backend;
use ast::FileAst;
use database::file_parse_output;
use typeck::{fn_call_at, position_to_byte_offset, signature_for_fn};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

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

        if let Some(state) = self.documents.get(&uri) {
            let snapshot = self.snapshot();
            let ast = file_parse_output(&snapshot.db, state.file).ast.clone();
            if let Some(help) = build_signature_help(&state.source, &ast, position) {
                return Ok(Some(help));
            }
        }

        Ok(None)
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
