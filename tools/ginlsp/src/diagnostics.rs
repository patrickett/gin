use ast::source::LineIndex;
use diagnostic::{Category, Diagnostic};
use resolve::find_import_suggestions;
use std::path::Path;
use tower_lsp::lsp_types::{
    Diagnostic as LspDiagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location,
    NumberOrString, Position, Range, Url,
};

pub struct DiagnosticConverter {
    source: String,
    line_index: LineIndex,
}

impl DiagnosticConverter {
    pub fn new(source: String, line_index: LineIndex) -> Self {
        Self { source, line_index }
    }

    fn lsp_diagnostic_message(&self, message: &str) -> String {
        message.trim().to_string()
    }

    fn diagnostic_related_information(
        &self,
        uri: &Url,
        range: Range,
        help_on_span: Option<&str>,
        help: Option<&str>,
    ) -> Option<Vec<DiagnosticRelatedInformation>> {
        let mut items = Vec::new();
        if let Some(s) = help_on_span.map(str::trim).filter(|s| !s.is_empty()) {
            items.push(DiagnosticRelatedInformation {
                location: Location {
                    uri: uri.clone(),
                    range,
                },
                message: s.to_string(),
            });
        }
        if let Some(h) = help.map(str::trim).filter(|s| !s.is_empty()) {
            items.push(DiagnosticRelatedInformation {
                location: Location {
                    uri: uri.clone(),
                    range,
                },
                message: format!("help: {h}"),
            });
        }
        (!items.is_empty()).then_some(items)
    }

    fn diagnostic_quickfix_data(
        &self,
        symptom: &Diagnostic,
        file_path: Option<&Path>,
    ) -> Option<serde_json::Value> {
        match symptom.code.slug() {
            "type-unknown-symbol" => {
                let name = symptom.arg("name")?;
                let did_you_mean = symptom.arg("suggested");
                if let Some(suggested) = did_you_mean {
                    Some(serde_json::json!({
                        "gincQuickFix": "replace-binding",
                        "oldName": name,
                        "newName": suggested,
                    }))
                } else {
                    let file_path = file_path?;
                    let suggestions = find_import_suggestions(file_path, name, &self.source);
                    if suggestions.is_empty() {
                        return None;
                    }
                    let suggestion_objs: Vec<serde_json::Value> = suggestions
                        .iter()
                        .map(|s| {
                            serde_json::json!({
                                "useLine": s.use_line,
                                "replaceLine": s.replace_line,
                                "deleteLines": s.delete_lines,
                            })
                        })
                        .collect();
                    Some(serde_json::json!({
                        "gincQuickFix": "add-import",
                        "symbol": name,
                        "suggestions": suggestion_objs,
                    }))
                }
            }
            "use-not-exported" => {
                let symbol = symptom.arg("symbol")?;
                Some(serde_json::json!({
                    "gincQuickFix": "remove-bundle-member",
                    "symbol": symbol,
                }))
            }
            "parse-indented-return" => Some(serde_json::json!({
                "gincQuickFix": "fix-indented-return",
            })),
            "use-prefer-member-import" => {
                let path_prefix = symptom.arg("pathPrefix")?;
                let symbol = symptom.arg("symbol")?;
                Some(serde_json::json!({
                    "gincQuickFix": "prefer-member-import",
                    "pathPrefix": path_prefix,
                    "symbol": symbol,
                }))
            }
            "parse-bind-name-not-snake-case" => {
                let name = symptom.arg("name")?;
                let suggested = symptom.arg("suggested")?;
                Some(serde_json::json!({
                    "gincQuickFix": "rename-bind-snake-case",
                    "oldName": name,
                    "newName": suggested,
                }))
            }
            "type-unreachable-else-arm" => Some(serde_json::json!({
                "gincQuickFix": "omit-unreachable-else",
            })),
            "parse-unnecessary-comma" => Some(serde_json::json!({
                "gincQuickFix": "remove-unnecessary-comma",
            })),
            _ => None,
        }
    }

    /// Convert a byte span to an LSP `Range` using the pre-computed `LineIndex`
    /// for fast O(log n) position conversion.
    pub fn span_to_range_indexed(&self, start: usize, end: usize) -> Range {
        let (start_line, start_col) = self.line_index.byte_to_position(&self.source, start);
        let (end_line, end_col) = self.line_index.byte_to_position(&self.source, end);

        let (line, character) = if start == end && start > 0 {
            self.line_index.byte_to_position(&self.source, start - 1)
        } else {
            (start_line, start_col)
        };

        Range {
            start: Position { line, character },
            end: Position {
                line: end_line,
                character: end_col,
            },
        }
    }

    /// Convert diagnostics to LSP diagnostics using the pre-computed `LineIndex`
    /// for fast per-span position conversion.
    ///
    /// When converting many diagnostics for the same file (e.g. during
    /// diagnostic publishing), this is significantly faster than the old
    /// per-symptom linear scan approach.
    pub fn symptoms_to_diagnostics(
        &self,
        symptoms: &[Diagnostic],
        document_uri: &Url,
        file_path: Option<&Path>,
    ) -> Vec<LspDiagnostic> {
        symptoms
            .iter()
            .map(|symptom| {
                let range = self.span_to_range_indexed(symptom.span.start(), symptom.span.end());
                let severity = match symptom.code.slug() {
                    "use-prefer-member-import" | "parse-bind-name-not-snake-case" => {
                        DiagnosticSeverity::WARNING
                    }
                    _ => match symptom.category {
                        Category::Flaw => DiagnosticSeverity::ERROR,
                        Category::Help => DiagnosticSeverity::HINT,
                        Category::Info => DiagnosticSeverity::INFORMATION,
                    },
                };
                let slug = symptom.error_code().to_string();
                let message = self.lsp_diagnostic_message(&symptom.message);
                let related_information = self.diagnostic_related_information(
                    document_uri,
                    range,
                    symptom.help_on_span.as_deref(),
                    symptom.help.as_deref(),
                );
                LspDiagnostic {
                    range,
                    severity: Some(severity),
                    code: Some(NumberOrString::String(slug)),
                    code_description: None,
                    source: Some("ginc".to_string()),
                    message,
                    related_information,
                    tags: None,
                    data: self.diagnostic_quickfix_data(symptom, file_path),
                }
            })
            .collect()
    }
}
#[cfg(test)]
#[path = "tests/diagnostics_tests.rs"]
mod tests;
