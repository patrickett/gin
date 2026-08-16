use crate::Backend;
use ast::source::SourceExt;
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

        if let Some(ref only) = params.context.only
            && !only.is_empty()
            && !only.iter().any(|k| {
                k.as_str() == CodeActionKind::QUICKFIX.as_str()
                    || k.as_str().starts_with("quickfix.")
            })
        {
            return Ok(Some(Vec::new()));
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

        let mut out: Vec<CodeActionOrCommand> = Vec::new();

        // Track diagnostics for "fix all" actions.
        let mut indented_return_diags: Vec<&Diagnostic> = Vec::new();
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
            if let Some(action) = Self::prefer_member_import_code_action(uri, diag, &source) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
            if let Some(action) = Self::fix_indented_return_code_action(uri, diag, &source) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
            if let Some(action) = Self::omit_unreachable_else_code_action(uri, diag, &source) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
            // Collect for "fix all" actions.
            if Self::is_indented_return_diag(diag) {
                indented_return_diags.push(diag);
            }
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
            .filter(|s| s.code.slug() == "parse-unnecessary-comma")
            .map(|s| {
                let (sl, sc) = source.byte_offset_to_position(s.span.start());
                let (el, ec) = source.byte_offset_to_position(s.span.end());
                Range {
                    start: Position {
                        line: sl,
                        character: sc,
                    },
                    end: Position {
                        line: el,
                        character: ec,
                    },
                }
            })
            .collect();

        if !all_comma_diags.is_empty()
            && let Some(action) =
                Self::remove_all_unnecessary_commas_code_action(uri, &all_comma_diags)
        {
            out.push(CodeActionOrCommand::CodeAction(action));
        }

        // Add "fix all" action if there are multiple indented returns.
        if indented_return_diags.len() > 1
            && let Some(action) =
                Self::fix_all_indented_returns_code_action(uri, &indented_return_diags, &source)
        {
            out.push(CodeActionOrCommand::CodeAction(action));
        }

        Ok(Some(out))
    }

    fn is_indented_return_diag(diag: &Diagnostic) -> bool {
        let v = match diag.data.as_ref() {
            Some(v) => v,
            None => return false,
        };
        let obj = match v.as_object() {
            Some(obj) => obj,
            None => return false,
        };
        obj.get("gincQuickFix").and_then(|x| x.as_str()) == Some("fix-indented-return")
    }

    /// Compute the text edit that unindents a return line to column 0.
    fn unindent_return_edit(diag: &Diagnostic, source: &str) -> Option<TextEdit> {
        let line = diag.range.start.line;
        let line_text = source.lines().nth(line as usize)?;

        // Find the first non-whitespace character on the line.
        let indent_end = line_text.len() - line_text.trim_start().len();
        if indent_end == 0 {
            return None; // already unindented
        }

        let range = Range {
            start: Position { line, character: 0 },
            end: Position {
                line,
                character: indent_end as u32,
            },
        };

        Some(TextEdit {
            range,
            new_text: String::new(), // remove leading whitespace
        })
    }

    fn omit_unreachable_else_edit(diag: &Diagnostic, source: &str) -> Option<TextEdit> {
        let start_byte =
            source.position_to_byte_offset(diag.range.start.line, diag.range.start.character)?;
        let end_byte =
            source.position_to_byte_offset(diag.range.end.line, diag.range.end.character)?;
        let line_start = source[..start_byte].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_prefix = &source[line_start..start_byte];
        let delete_start = if line_prefix.chars().all(|c| matches!(c, ' ' | '\t')) {
            line_start
        } else {
            start_byte
        };
        Some(TextEdit {
            range: Self::byte_range_to_range(delete_start, end_byte, source),
            new_text: String::new(),
        })
    }

    fn omit_unreachable_else_code_action(
        uri: &Url,
        diag: &Diagnostic,
        source: &str,
    ) -> Option<CodeAction> {
        let v = diag.data.as_ref()?;
        let obj = v.as_object()?;
        if obj.get("gincQuickFix").and_then(|x| x.as_str()) != Some("omit-unreachable-else") {
            return None;
        }
        let edit = Self::omit_unreachable_else_edit(diag, source)?;

        let mut changes = HashMap::new();
        changes.insert(uri.clone(), vec![edit]);

        Some(CodeAction {
            title: "Remove unreachable `else` clause".to_string(),
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

    fn fix_indented_return_code_action(
        uri: &Url,
        diag: &Diagnostic,
        source: &str,
    ) -> Option<CodeAction> {
        if !Self::is_indented_return_diag(diag) {
            return None;
        }
        let edit = Self::unindent_return_edit(diag, source)?;

        let mut changes = HashMap::new();
        changes.insert(uri.clone(), vec![edit]);

        Some(CodeAction {
            title: "Unindent return".to_string(),
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

    fn fix_all_indented_returns_code_action(
        uri: &Url,
        diags: &[&Diagnostic],
        source: &str,
    ) -> Option<CodeAction> {
        let mut changes = HashMap::new();
        let mut edits: Vec<TextEdit> = Vec::new();

        for diag in diags {
            if let Some(edit) = Self::unindent_return_edit(diag, source) {
                edits.push(edit);
            }
        }

        if edits.is_empty() {
            return None;
        }

        changes.insert(uri.clone(), edits);

        Some(CodeAction {
            title: format!("Unindent all {} returns", diags.len()),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(diags.iter().map(|d| (*d).clone()).collect()),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            is_preferred: Some(false),
            ..Default::default()
        })
    }

    fn prefer_member_import_code_action(
        uri: &Url,
        diag: &Diagnostic,
        source: &str,
    ) -> Option<CodeAction> {
        let v = diag.data.as_ref()?;
        let obj = v.as_object()?;
        if obj.get("gincQuickFix").and_then(|x| x.as_str()) != Some("prefer-member-import") {
            return None;
        }
        let symbol = obj.get("symbol").and_then(|x| x.as_str())?;

        let line = diag.range.start.line;
        let line_text = source.lines().nth(line as usize)?;
        let old = format!(".({symbol})");
        let new = format!(".{symbol}");
        let start_byte = line_text.find(&old)?;
        let end_byte = start_byte + old.len();

        let range = Range {
            start: Position {
                line,
                character: start_byte as u32,
            },
            end: Position {
                line,
                character: end_byte as u32,
            },
        };

        let mut changes = HashMap::new();
        changes.insert(
            uri.clone(),
            vec![TextEdit {
                range,
                new_text: new,
            }],
        );

        Some(CodeAction {
            title: format!("Use `.{symbol}` member import"),
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

    fn sort_imports_code_action(uri: &Url, diag: &Diagnostic) -> Option<CodeAction> {
        let v = diag.data.as_ref()?;
        let obj = v.as_object()?;
        if obj.get("gincQuickFix").and_then(|x| x.as_str()) != Some("sort-imports") {
            return None;
        }
        let sorted_text = obj.get("sortedText").and_then(|x| x.as_str())?;

        let mut changes = HashMap::new();
        changes.insert(
            uri.clone(),
            vec![TextEdit {
                range: diag.range,
                new_text: sorted_text.to_string(),
            }],
        );

        Some(CodeAction {
            title: "Sort imports alphabetically".to_string(),
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

    /// Byte/line position to insert a new `use` line (after the last `use` block, or at file start).
    fn use_insert_position(source: &str) -> Position {
        let mut last_use_line: Option<u32> = None;
        for (i, line) in source.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("use ") {
                last_use_line = Some(i as u32);
            } else if !t.is_empty() && !t.starts_with("--") && last_use_line.is_some() {
                break;
            }
        }
        match last_use_line {
            Some(line) => Position {
                line: line + 1,
                character: 0,
            },
            None => Position {
                line: 0,
                character: 0,
            },
        }
    }

    fn line_byte_range(source: &str, line: u32) -> Option<(usize, usize)> {
        let mut line_start = 0usize;
        let mut current = 0u32;
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                if current == line {
                    return Some((line_start, i + 1));
                }
                current += 1;
                line_start = i + 1;
            }
        }
        if current == line {
            return Some((line_start, source.len()));
        }
        None
    }

    fn byte_range_to_range(start: usize, end: usize, source: &str) -> Range {
        let (sl, sc) = source.byte_offset_to_position(start);
        let (el, ec) = source.byte_offset_to_position(end);
        Range {
            start: Position {
                line: sl,
                character: sc,
            },
            end: Position {
                line: el,
                character: ec,
            },
        }
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
            if let Some((start, end)) = Self::line_byte_range(source, line) {
                edits.push(TextEdit {
                    range: Self::byte_range_to_range(start, end, source),
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
            if let Some((start, end)) = Self::line_byte_range(source, line) {
                edits.push(TextEdit {
                    range: Self::byte_range_to_range(start, end, source),
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
        let v = diag.data.as_ref()?;
        let obj = v.as_object()?;
        if obj.get("gincQuickFix").and_then(|x| x.as_str()) != Some("add-import") {
            return None;
        }
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
            let mut changes = HashMap::new();
            changes.insert(uri.clone(), text_edits);
            actions.push(CodeAction {
                title: format!("Add `{use_line}`"),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diag.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                is_preferred: Some(actions.is_empty()),
                ..Default::default()
            });
        }

        (!actions.is_empty()).then_some(actions)
    }

    fn rename_bind_snake_case_code_action(uri: &Url, diag: &Diagnostic) -> Option<CodeAction> {
        let v = diag.data.as_ref()?;
        let obj = v.as_object()?;
        if obj.get("gincQuickFix").and_then(|x| x.as_str()) != Some("rename-bind-snake-case") {
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
            title: format!("Rename bind `{old_name}` to `{new_name}`"),
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

    /// Check if a diagnostic is an unnecessary-comma hint.
    fn is_unnecessary_comma_diag(diag: &Diagnostic) -> bool {
        let v = match diag.data.as_ref() {
            Some(v) => v,
            None => return false,
        };
        let obj = match v.as_object() {
            Some(obj) => obj,
            None => return false,
        };
        obj.get("gincQuickFix").and_then(|x| x.as_str()) == Some("remove-unnecessary-comma")
    }

    /// Compute the text edit that removes an unnecessary comma.
    fn remove_unnecessary_comma_edit(diag: &Diagnostic, source: &str) -> Option<TextEdit> {
        let start_byte =
            source.position_to_byte_offset(diag.range.start.line, diag.range.start.character)?;
        let end_byte =
            source.position_to_byte_offset(diag.range.end.line, diag.range.end.character)?;

        Some(TextEdit {
            range: Self::byte_range_to_range(start_byte, end_byte, source),
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

        let mut changes = HashMap::new();
        changes.insert(uri.clone(), vec![edit]);

        Some(CodeAction {
            title: "Remove unnecessary comma".to_string(),
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
