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

        let source = self
            .documents
            .get(uri.as_str())
            .map(|d| d.source.clone())
            .unwrap_or_default();

        let mut out: Vec<CodeActionOrCommand> = Vec::new();
        for diag in &params.context.diagnostics {
            if let Some(action) = replace_binding_code_action(uri, diag) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
            if let Some(action) = remove_bundle_member_code_action(uri, diag, &source) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
        }

        Ok(Some(out))
    }
}

fn remove_bundle_member_code_action(
    uri: &Url,
    diag: &Diagnostic,
    source: &str,
) -> Option<CodeAction> {
    let v = diag.data.as_ref()?;
    let obj = v.as_object()?;
    if obj.get("gincQuickFix").and_then(|x| x.as_str()) != Some("remove-bundle-member") {
        return None;
    }
    let symbol = obj.get("symbol").and_then(|x| x.as_str())?;

    let line = diag.range.start.line;
    let start_char = diag.range.start.character as usize;
    let end_char = diag.range.end.character as usize;
    let line_text = source.lines().nth(line as usize)?;
    let before = &line_text[..start_char];
    let after = &line_text[end_char..];

    let (remove_start, remove_end) = if before.ends_with(", ") {
        (start_char - 2, end_char)
    } else if after.starts_with(", ") {
        (start_char, end_char + 2)
    } else if before.ends_with(',') {
        (start_char - 1, end_char)
    } else if after.starts_with(',') {
        (start_char, end_char + 1)
    } else {
        (start_char, end_char)
    };

    let range = Range {
        start: Position {
            line,
            character: remove_start as u32,
        },
        end: Position {
            line,
            character: remove_end as u32,
        },
    };

    let mut changes = HashMap::new();
    changes.insert(
        uri.clone(),
        vec![TextEdit {
            range,
            new_text: String::new(),
        }],
    );

    Some(CodeAction {
        title: format!("Remove `{symbol}`"),
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
