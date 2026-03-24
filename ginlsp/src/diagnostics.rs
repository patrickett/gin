use ginc::{Category, Symptom, typeck::FlowAnalysis};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, DiagnosticTag, NumberOrString, Position, Range};

pub fn span_to_range(start: usize, end: usize, source: &str) -> Range {
    let (start_line, start_col) = byte_to_position(start, source);
    let (end_line, end_col) = byte_to_position(end, source);

    Range {
        start: Position {
            line: start_line,
            character: start_col,
        },
        end: Position {
            line: end_line,
            character: end_col,
        },
    }
}

fn byte_to_position(byte: usize, source: &str) -> (u32, u32) {
    let mut line = 0u32;
    let mut col = 0u32;
    let mut current_byte = 0usize;

    for ch in source.chars() {
        if current_byte >= byte {
            break;
        }

        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += ch.len_utf8() as u32;
        }
        current_byte += ch.len_utf8();
    }

    (line, col)
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
                code: Some(NumberOrString::String(symptom.error_code())),
                code_description: None,
                source: Some("ginc".to_string()),
                message: symptom.message(),
                related_information: None,
                tags: None,
                data: None,
            }
        })
        .collect()
}

/// Convert flow analysis impossible checks to LSP diagnostics.
pub fn flow_analysis_to_diagnostics(flow_result: &FlowAnalysis) -> Vec<Diagnostic> {
    flow_result
        .impossible_checks
        .iter()
        .map(|check| Diagnostic {
            // TODO: Map expr_index to actual position in source
            // For now, use a zero-range as placeholder
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 0 },
            },
            severity: Some(DiagnosticSeverity::WARNING),
            code: Some(NumberOrString::String("FLOW001".to_string())),
            code_description: None,
            source: Some("gin-flow".to_string()),
            message: check.reason.clone(),
            related_information: None,
            tags: Some(vec![DiagnosticTag::UNNECESSARY]),
            data: None,
        })
        .collect()
}
