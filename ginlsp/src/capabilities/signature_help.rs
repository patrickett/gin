use ginc::FileAst;
use tower_lsp::lsp_types::{
    Documentation, MarkupContent, MarkupKind, ParameterInformation, ParameterLabel, SignatureHelp,
    SignatureInformation,
};

pub fn build_signature_help(
    source: &str,
    ast: &FileAst,
    position: tower_lsp::lsp_types::Position,
) -> Option<SignatureHelp> {
    let byte_pos = ginc::position_to_byte_offset(source, position.line, position.character)?;
    let fn_name = ginc::fn_call_at(ast, byte_pos)?;
    let info = ginc::signature_for_fn(ast, &fn_name)?;

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
