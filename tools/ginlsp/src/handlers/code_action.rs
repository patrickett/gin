use crate::Backend;
use crate::handlers::{byte_range_to_range, line_byte_range, single_file_action};
use ast::HasSpanId;
use ast::source::SourceExt;
use diagnostic::DiagnosticCode;
use diagnostic::code::{ParseCode, SyntaxCode};
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
        if !Backend::should_handle_file(uri) {
            return Ok(None);
        }

        let mut out = self.style_code_actions(&params);

        if let Some(ref only) = params.context.only
            && !only.is_empty()
            && !only.iter().any(|k| {
                k.as_str() == CodeActionKind::QUICKFIX.as_str()
                    || k.as_str().starts_with("quickfix.")
            })
        {
            return Ok(Some(out));
        }

        // Read source and parse output from the engine (single source of truth).
        let (source, engine_symptoms) = Backend::file_path_from_uri(uri)
            .and_then(|path| {
                let st = self.snapshot();
                st.cache
                    .source_and_parse(&path)
                    .map(|(s, parse)| (s, parse.symptoms.clone()))
            })
            .unzip();
        let source: std::sync::Arc<String> =
            source.unwrap_or_else(|| std::sync::Arc::new(String::new()));
        let engine_symptoms: Vec<diagnostic::Diagnostic> = engine_symptoms.unwrap_or_default();

        // Track diagnostics for "fix all" actions.
        let mut cursor_comma_diags: Vec<&Diagnostic> = Vec::new();

        for diag in &params.context.diagnostics {
            if let Some(action) = Self::replace_binding_code_action(uri, diag) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
            if let Some(action) = Self::rename_bind_snake_case_code_action(uri, diag) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
            if let Some(actions) = Self::add_import_code_actions(uri, diag, &source) {
                for action in actions {
                    out.push(CodeActionOrCommand::CodeAction(action));
                }
            }
            if let Some(action) = Self::remove_bundle_member_code_action(uri, diag, &source) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
            if let Some(action) = Self::sort_imports_code_action(uri, diag) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
            if let Some(action) = Self::omit_unreachable_else_code_action(uri, diag, &source) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
            // Collect for "fix all" actions.
            if Self::is_unnecessary_comma_diag(diag) {
                cursor_comma_diags.push(diag);
            }
        }

        // Single "Remove unnecessary comma" action — uses the cursor-context diagnostic.
        if let Some(first) = cursor_comma_diags.first()
            && let Some(action) = Self::remove_unnecessary_comma_code_action(uri, first, &source)
        {
            out.push(CodeActionOrCommand::CodeAction(action));
        }

        // Collect ALL unnecessary comma diagnostics from the engine for the bulk action.
        let all_comma_diags: Vec<Range> = engine_symptoms
            .iter()
            .filter(|s| {
                s.code == DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::UnnecessaryComma))
            })
            .map(|s| byte_range_to_range(s.span.start(), s.span.end(), &source))
            .collect();

        if !all_comma_diags.is_empty()
            && let Some(action) =
                Self::remove_all_unnecessary_commas_code_action(uri, &all_comma_diags)
        {
            out.push(CodeActionOrCommand::CodeAction(action));
        }

        Ok(Some(out))
    }

    fn quickfix(title: &str, diag: &Diagnostic, uri: &Url, edits: Vec<TextEdit>) -> CodeAction {
        let mut action =
            single_file_action(uri, title.to_string(), CodeActionKind::QUICKFIX, edits);
        action.diagnostics = Some(vec![diag.clone()]);
        action
    }

    fn quickfix_obj<'a>(
        diag: &'a Diagnostic,
        command: &str,
    ) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
        let obj = diag.data.as_ref()?.as_object()?;
        (obj.get("gincQuickFix").and_then(|x| x.as_str()) == Some(command)).then_some(obj)
    }

    fn omit_unreachable_else_edit(diag: &Diagnostic, source: &str) -> Option<TextEdit> {
        let start_byte =
            source.position_to_byte_offset(diag.range.start.line, diag.range.start.character)?;
        let end_byte =
            source.position_to_byte_offset(diag.range.end.line, diag.range.end.character)?;
        let (line_start, _) = line_byte_range(source, diag.range.start.line)?;
        let line_prefix = &source[line_start..start_byte];
        let delete_start = if line_prefix.chars().all(|c| matches!(c, ' ' | '\t')) {
            line_start
        } else {
            start_byte
        };
        Some(TextEdit {
            range: byte_range_to_range(delete_start, end_byte, source),
            new_text: String::new(),
        })
    }

    fn omit_unreachable_else_code_action(
        uri: &Url,
        diag: &Diagnostic,
        source: &str,
    ) -> Option<CodeAction> {
        Self::quickfix_obj(diag, "omit-unreachable-else")?;
        let edit = Self::omit_unreachable_else_edit(diag, source)?;

        Some(Self::quickfix(
            "Remove unreachable `else` clause",
            diag,
            uri,
            vec![edit],
        ))
    }

    fn sort_imports_code_action(uri: &Url, diag: &Diagnostic) -> Option<CodeAction> {
        let obj = Self::quickfix_obj(diag, "sort-imports")?;
        let sorted_text = obj.get("sortedText").and_then(|x| x.as_str())?;

        Some(Self::quickfix(
            "Sort imports alphabetically",
            diag,
            uri,
            vec![TextEdit {
                range: diag.range,
                new_text: sorted_text.to_string(),
            }],
        ))
    }

    fn remove_bundle_member_code_action(
        uri: &Url,
        diag: &Diagnostic,
        source: &str,
    ) -> Option<CodeAction> {
        let obj = Self::quickfix_obj(diag, "remove-bundle-member")?;
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

        Some(Self::quickfix(
            &format!("Remove `{symbol}`"),
            diag,
            uri,
            vec![TextEdit {
                range,
                new_text: String::new(),
            }],
        ))
    }

    /// Byte/line position to insert a new `use` line (after the last `use` block, or at file start).
    fn use_insert_position(source: &str) -> Position {
        let parsed = parser::query::SourceParseExt::parse_source_full(source);
        let spans = &parsed.ast.span_table;
        let last = parsed
            .ast
            .uses
            .iter()
            .flat_map(|import| &import.0)
            .map(|module| {
                let span = match &module.source {
                    ast::ImportSource::LocalBundle(b) => {
                        b.members.last().map_or(b.span, |m| m.span)
                    }
                    _ => module.source.span_id(),
                };
                spans.get(span).end()
            })
            .max();
        last.map_or(Position::new(0, 0), |end| {
            let (line, _) = source.byte_offset_to_position(end.saturating_sub(1));
            Position::new(line + 1, 0)
        })
    }

    fn build_add_import_edits(
        source: &str,
        use_line: &str,
        replace_line: Option<u32>,
        delete_lines: &[u32],
        insert_pos: Position,
        needs_leading_newline: bool,
    ) -> Vec<TextEdit> {
        let mut edits = Vec::new();

        if let Some(line) = replace_line {
            if let Some((start, end)) = line_byte_range(source, line) {
                edits.push(TextEdit {
                    range: byte_range_to_range(start, end, source),
                    new_text: format!("{use_line}\n"),
                });
            }
        } else {
            let new_text = if needs_leading_newline {
                format!("\n{use_line}\n")
            } else {
                format!("{use_line}\n")
            };
            edits.push(TextEdit {
                range: Range {
                    start: insert_pos,
                    end: insert_pos,
                },
                new_text,
            });
        }

        let mut sorted_deletes: Vec<u32> = delete_lines.to_vec();
        sorted_deletes.sort_by(|a, b| b.cmp(a));
        for line in sorted_deletes {
            if Some(line) == replace_line {
                continue;
            }
            if let Some((start, end)) = line_byte_range(source, line) {
                edits.push(TextEdit {
                    range: byte_range_to_range(start, end, source),
                    new_text: String::new(),
                });
            }
        }

        edits
    }

    fn add_import_code_actions(
        uri: &Url,
        diag: &Diagnostic,
        source: &str,
    ) -> Option<Vec<CodeAction>> {
        let obj = Self::quickfix_obj(diag, "add-import")?;
        let suggestions = obj.get("suggestions")?.as_array()?;

        let insert_pos = Self::use_insert_position(source);
        let needs_leading_newline = insert_pos.line > 0
            && source
                .lines()
                .nth(insert_pos.line as usize)
                .is_some_and(|l| !l.is_empty());

        let mut actions = Vec::new();
        for suggestion in suggestions {
            let s = suggestion.as_object()?;
            let use_line = s.get("useLine")?.as_str()?;
            let replace_line = s
                .get("replaceLine")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32);
            let delete_lines: Vec<u32> = s
                .get("deleteLines")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_u64().map(|n| n as u32))
                        .collect()
                })
                .unwrap_or_default();

            let text_edits = Self::build_add_import_edits(
                source,
                use_line,
                replace_line,
                &delete_lines,
                insert_pos,
                needs_leading_newline,
            );
            let mut action = Self::quickfix(&format!("Add `{use_line}`"), diag, uri, text_edits);
            action.is_preferred = Some(actions.is_empty());
            actions.push(action);
        }

        (!actions.is_empty()).then_some(actions)
    }

    fn rename_bind_snake_case_code_action(uri: &Url, diag: &Diagnostic) -> Option<CodeAction> {
        let obj = Self::quickfix_obj(diag, "rename-bind-snake-case")?;
        let old_name = obj.get("oldName").and_then(|x| x.as_str())?;
        let new_name = obj.get("newName").and_then(|x| x.as_str())?;

        Some(Self::quickfix(
            &format!("Rename bind `{old_name}` to `{new_name}`"),
            diag,
            uri,
            vec![TextEdit {
                range: diag.range,
                new_text: new_name.to_string(),
            }],
        ))
    }

    fn replace_binding_code_action(uri: &Url, diag: &Diagnostic) -> Option<CodeAction> {
        let obj = Self::quickfix_obj(diag, "replace-binding")?;
        let old_name = obj.get("oldName").and_then(|x| x.as_str())?;
        let new_name = obj.get("newName").and_then(|x| x.as_str())?;

        Some(Self::quickfix(
            &format!("Replace `{old_name}` with `{new_name}`"),
            diag,
            uri,
            vec![TextEdit {
                range: diag.range,
                new_text: new_name.to_string(),
            }],
        ))
    }

    /// Check if a diagnostic is an unnecessary-comma hint.
    fn is_unnecessary_comma_diag(diag: &Diagnostic) -> bool {
        Self::quickfix_obj(diag, "remove-unnecessary-comma").is_some()
    }

    /// Compute the text edit that removes an unnecessary comma.
    fn remove_unnecessary_comma_edit(diag: &Diagnostic, source: &str) -> Option<TextEdit> {
        let start_byte =
            source.position_to_byte_offset(diag.range.start.line, diag.range.start.character)?;
        let end_byte =
            source.position_to_byte_offset(diag.range.end.line, diag.range.end.character)?;

        Some(TextEdit {
            range: byte_range_to_range(start_byte, end_byte, source),
            new_text: String::new(),
        })
    }

    /// Code action to remove a single unnecessary comma.
    fn remove_unnecessary_comma_code_action(
        uri: &Url,
        diag: &Diagnostic,
        source: &str,
    ) -> Option<CodeAction> {
        if !Self::is_unnecessary_comma_diag(diag) {
            return None;
        }
        let edit = Self::remove_unnecessary_comma_edit(diag, source)?;

        Some(Self::quickfix(
            "Remove unnecessary comma",
            diag,
            uri,
            vec![edit],
        ))
    }

    /// Code action to remove all unnecessary commas in the file.
    fn remove_all_unnecessary_commas_code_action(
        uri: &Url,
        ranges: &[Range],
    ) -> Option<CodeAction> {
        let mut changes = HashMap::new();
        let mut edits: Vec<TextEdit> = Vec::new();

        for range in ranges {
            edits.push(TextEdit {
                range: *range,
                new_text: String::new(),
            });
        }

        if edits.is_empty() {
            return None;
        }

        changes.insert(uri.clone(), edits);

        Some(CodeAction {
            title: "Remove all unnecessary commas".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: None,
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            is_preferred: Some(false),
            ..Default::default()
        })
    }
}

#[cfg(test)]
#[path = "tests/add_import_tests.rs"]
mod add_import_tests;
