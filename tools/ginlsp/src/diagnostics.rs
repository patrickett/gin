use diagnostic::{Category, Symptom};
use lsp::source::byte_offset_to_position;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};

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

pub fn symptoms_to_diagnostics(source: &str, symptoms: &[&Symptom]) -> Vec<Diagnostic> {
    symptoms
        .iter()
        .map(|symptom| {
            let range = span_to_range(symptom.span.start, symptom.span.end, source);
            let severity = match symptom.category {
                Category::Flaw => DiagnosticSeverity::ERROR,
                Category::Help => DiagnosticSeverity::HINT,
                Category::Info => DiagnosticSeverity::INFORMATION,
            };
            Diagnostic {
                range,
                severity: Some(severity),
                code: Some(NumberOrString::String(symptom.error_code().to_string())),
                code_description: None,
                source: Some("ginc".to_string()),
                message: symptom.message.clone(),
                related_information: None,
                tags: None,
                data: None,
            }
        })
        .collect()
}
