//! A flaw or warning of the shared flaw path as an LSP diagnostic.
//!
//! * range: the culprit (the exact expression being checked) when the checker names one, else
//!   the enclosing construct, else the start of the file;
//! * related information: the enclosing construct, when the range is narrower than it;
//! * message: the checker's message, then what it had proved ("known here: ...") and, after
//!   a declaration was set aside, a note that the flaw may be a consequence of it;
//! * data: everything structured (`code`, `stage`, `claim`, `construct`, `facts`,
//!   `knock_on_of`, and for a hole its expected type) for clients and code actions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ginterpreter::check::HoleInfo;
use ginterpreter::flaw::{Flaw, fact_json, fact_text};
use lsp_types::{Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString, Url};
use serde_json::json;

use crate::analysis::Analysis;
use crate::position::LineIndex;

pub const SOURCE: &str = "ginlsp";
pub const HOLE_CODE: &str = "source-check-hole";

pub type UriOf<'a> = &'a dyn Fn(&Path) -> Option<Url>;

/// Diagnostics of an analysis, by the file they belong to. A flaw with no file of its own (a
/// package that does not load) is shown on `trigger`.
pub fn convert(analysis: &Analysis, trigger: &Path, uri_of: UriOf<'_>) -> BTreeMap<PathBuf, Vec<Diagnostic>> {
    let mut out: BTreeMap<PathBuf, Vec<Diagnostic>> = BTreeMap::new();
    for flaw in analysis.checked.all() {
        let path = if flaw.file == analysis.checked.file { trigger.to_owned() } else { PathBuf::from(&flaw.file) };
        let hole = (flaw.code == HOLE_CODE).then(|| find_hole(&analysis.checked.holes, flaw)).flatten();
        out.entry(path).or_default().push(diagnostic(flaw, hole, uri_of));
    }
    for diagnostics in out.values_mut() {
        diagnostics.sort_by_key(|d| (d.range.start.line, d.range.start.character, d.range.end.line, d.range.end.character));
    }
    out
}

fn find_hole<'a>(holes: &'a [HoleInfo], flaw: &Flaw) -> Option<&'a HoleInfo> {
    let span = flaw.span.as_ref()?;
    holes.iter().find(|h| h.span.file == span.file && h.span.byte_start == span.byte_start)
}

pub fn diagnostic(flaw: &Flaw, hole: Option<&HoleInfo>, uri_of: UriOf<'_>) -> Diagnostic {
    let text: &str = &flaw.source;
    let index = LineIndex::new(text);
    let primary = flaw.culprit.as_ref().or(flaw.span.as_ref());
    let range = match primary {
        Some(located) => index.range(text, located.byte_start, located.byte_end),
        None => index.range(text, 0, 0),
    };
    let related_information = match (&flaw.culprit, &flaw.span) {
        (Some(culprit), Some(span)) if (culprit.byte_start, culprit.byte_end) != (span.byte_start, span.byte_end) => {
            uri_of(Path::new(&flaw.file)).map(|uri| {
                vec![DiagnosticRelatedInformation {
                    location: Location { uri, range: index.range(text, span.byte_start, span.byte_end) },
                    message: match &flaw.construct {
                        Some(c) => format!("enclosing source construct: {} `{}`", c.kind, c.name),
                        None => "enclosing source construct".to_owned(),
                    },
                }]
            })
        }
        _ => None,
    };
    let claims: Vec<String> = flaw.proved.iter().flatten().map(fact_text).collect();
    let mut message = match hole {
        Some(hole) => hole_message(hole),
        None => flaw.message.clone(),
    };
    if hole.is_none() && !claims.is_empty() {
        message.push_str(&format!("\nknown here: {}", claims.join("; ")));
    }
    if !flaw.knock_on_of.is_empty() {
        message.push_str(&format!("\nmay follow from the flaw in: {}", flaw.knock_on_of.join(", ")));
    }
    let severity = if flaw.is_flaw() {
        DiagnosticSeverity::ERROR
    } else if flaw.code == HOLE_CODE {
        DiagnosticSeverity::INFORMATION
    } else {
        DiagnosticSeverity::WARNING
    };
    let data = json!({
        "code": flaw.code,
        "stage": flaw.stage,
        "claim": flaw.claim,
        "construct": flaw.construct.as_ref().map(|c| json!({"kind": c.kind, "name": c.name, "result": c.result})),
        "facts": claims,
        "proved": flaw.proved.as_ref().map(|p| p.iter().map(fact_json).collect::<Vec<_>>()),
        "knock_on_of": flaw.knock_on_of,
        "hole": hole.map(|h| json!({"expected_type": h.expected, "construct": h.construct, "facts": h.facts})),
    });
    Diagnostic {
        range,
        severity: Some(severity),
        code: Some(NumberOrString::String(flaw.code.clone())),
        code_description: None,
        source: Some(SOURCE.to_owned()),
        message,
        related_information,
        tags: None,
        data: Some(data),
    }
}

fn hole_message(hole: &HoleInfo) -> String {
    let mut message = match &hole.expected {
        Some(expected) => format!("`#todo` hole in `{}`: expected a value of type `{expected}`", hole.construct),
        None => format!("`#todo` hole in `{}`", hole.construct),
    };
    if !hole.facts.is_empty() {
        message.push_str(&format!("\nknown here: {}", hole.facts.join("; ")));
    }
    message
}
