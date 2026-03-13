use crate::capabilities::completion::extract_fn_name_before_paren;
use crate::util::format_params;
use ginc::FileAst;
use ropey::Rope;
use tower_lsp::lsp_types::{
    Documentation, MarkupContent, MarkupKind, ParameterInformation, ParameterLabel, SignatureHelp,
    SignatureInformation,
};

pub fn build_signature_help(
    source: &str,
    ast: &FileAst,
    position: tower_lsp::lsp_types::Position,
) -> Option<SignatureHelp> {
    let rope = Rope::from_str(source);
    let line_start = rope
        .try_line_to_char(position.line as usize)
        .ok()
        .and_then(|lc| rope.try_char_to_byte(lc).ok())?;

    let cursor_byte = line_start + position.character as usize;
    let line_text = &source[line_start..cursor_byte.min(source.len())];

    let fn_name = extract_fn_name_before_paren(line_text)?;

    for (name, bind) in ast.defs() {
        if name.as_str() == fn_name {
            if let Some(params) = bind.params() {
                let param_infos: Vec<ParameterInformation> = params
                    .keys()
                    .map(|p| ParameterInformation {
                        label: ParameterLabel::Simple(p.to_string()),
                        documentation: None,
                    })
                    .collect();

                let sig = SignatureInformation {
                    label: format!("{}{}", fn_name, format_params(params)),
                    documentation: bind.doc_comment().map(|dc| {
                        Documentation::MarkupContent(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: dc.0.clone(),
                        })
                    }),
                    parameters: Some(param_infos),
                    active_parameter: None,
                };

                return Some(SignatureHelp {
                    signatures: vec![sig],
                    active_signature: Some(0),
                    active_parameter: None,
                });
            }
        }
    }

    None
}
