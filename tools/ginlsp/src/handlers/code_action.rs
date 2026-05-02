use crate::Backend;
use std::collections::HashMap;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_code_action(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<CodeActionResponse>> {
        if self.is_shutdown() {
            return Ok(None);
        }

        let uri = &params.text_document.uri;
        if !super::should_handle_file(uri) {
            return Ok(None);
        }

        if let Some(ref only) = params.context.only {
            if !only.is_empty()
                && !only.iter().any(|k| {
                    k.as_str() == CodeActionKind::QUICKFIX.as_str()
                        || k.as_str().starts_with("quickfix.")
                })
            {
                return Ok(Some(Vec::new()));
            }
        }

        let mut out: Vec<CodeActionOrCommand> = Vec::new();
        for diag in &params.context.diagnostics {
            if let Some(action) = replace_binding_code_action(uri, diag) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
        }

        Ok(Some(out))
    }
}

fn replace_binding_code_action(uri: &Url, diag: &Diagnostic) -> Option<CodeAction> {
    let v = diag.data.as_ref()?;
    let obj = v.as_object()?;
    if obj.get("gincQuickFix").and_then(|x| x.as_str()) != Some("replace-binding") {
        return None;
    }
    let old_name = obj.get("oldName").and_then(|x| x.as_str())?;
    let new_name = obj.get("newName").and_then(|x| x.as_str())?;

    let mut changes = HashMap::new();
    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range: diag.range,
            new_text: new_name.to_string(),
        }],
    );

    Some(CodeAction {
        title: format!("Replace `{old_name}` with `{new_name}`"),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        is_preferred: Some(true),
        ..Default::default()
    })
}
