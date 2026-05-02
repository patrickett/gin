use database::Diagnostics;
use diagnostic::{Category, DiagnosticCode, TypeSymptom};
use span::SpanTable;
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString,
    Position, Range, Url,
};
use typeck::byte_offset_to_position;

/// One-line diagnostic text for the editor (Problems / hover): primary message plus
/// `ginc(<slug>)`. Details live in `related_information`.
fn lsp_diagnostic_message(message: &str, code_slug: &str) -> String {
    format!("{} ginc({})", message.trim(), code_slug)
}

fn diagnostic_related_information(
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

fn diagnostic_quickfix_data(symptom: &Diagnostics) -> Option<serde_json::Value> {
    match &symptom.code {
        DiagnosticCode::Type(TypeSymptom::UnknownBinding {
            name,
            did_you_mean: Some(suggested),
        }) => Some(serde_json::json!({
            "gincQuickFix": "replace-binding",
            "oldName": name,
            "newName": suggested,
        })),
        _ => None,
    }
}

pub fn span_to_range(start: usize, end: usize, source: &str) -> Range {
    let (start_line, start_col) = byte_offset_to_position(start, source);
    let (end_line, end_col) = byte_offset_to_position(end, source);

    // Handle zero-length spans by including the previous character
    let (start, start_char) = if start == end && start > 0 {
        let (prev_line, prev_col) = byte_offset_to_position(start - 1, source);
        (prev_line, prev_col)
    } else {
        (start_line, start_col)
    };

    Range {
        start: Position {
            line: start,
            character: start_char,
        },
        end: Position {
            line: end_line,
            character: end_col,
        },
    }
}

pub fn symptoms_to_diagnostics(
    source: &str,
    span_table: &SpanTable,
    symptoms: &[&Diagnostics],
    document_uri: &Url,
) -> Vec<Diagnostic> {
    symptoms
        .iter()
        .map(|symptom| {
            let span = span_table.get(symptom.span_id);
            let range = span_to_range(span.start, span.end, source);
            let severity = match symptom.category {
                Category::Flaw => DiagnosticSeverity::ERROR,
                Category::Help => DiagnosticSeverity::HINT,
                Category::Info => DiagnosticSeverity::INFORMATION,
            };
            let slug = symptom.error_code().to_string();
            let message = lsp_diagnostic_message(&symptom.message, &slug);
            let related_information = diagnostic_related_information(
                document_uri,
                range.clone(),
                symptom.help_on_span.as_deref(),
                symptom.help.as_deref(),
            );
            Diagnostic {
                range,
                severity: Some(severity),
                code: Some(NumberOrString::String(slug)),
                code_description: None,
                source: Some("ginc".to_string()),
                message,
                related_information,
                tags: None,
                data: diagnostic_quickfix_data(symptom),
            }
        })
        .collect()
}
