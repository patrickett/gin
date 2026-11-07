//! One definition of how a new-checker failure or note becomes a diagnostic. `--check-new`, the
//! REPL and the JSONL protocol of `ginterpreter` all build theirs here, so they cannot
//! diverge: the same code, message, primary span, label and facts.

use diagnostic::{Diagnostic, RelatedSpan, Span};
use typecheck::{SourceCheckFailure, SourceCheckNote, SourceFact};

use crate::pipeline::{failure_details, note_code};

/// A proved fact as the claim it makes: `0 <= n <= 9`, `k = 7`, `n < m`.
pub fn fact_claim(fact: &SourceFact) -> String {
    match fact {
        SourceFact::Interval { name, lower, upper } => format!("{lower} <= {name} <= {upper}"),
        SourceFact::Known { name, value } => format!("{name} = {value}"),
        SourceFact::Multiple { name, modulus } => format!("{name} % {modulus} = 0"),
        SourceFact::Offset { name, base, offset } => format!("{name} = {base} + {offset}"),
        SourceFact::Relation {
            left,
            operator,
            right,
        } => format!("{left} {operator} {right}"),
    }
}

fn with_facts(mut diagnostic: Diagnostic, facts: &[SourceFact]) -> Diagnostic {
    if facts.is_empty() {
        return diagnostic;
    }
    let claims: Vec<String> = facts.iter().map(fact_claim).collect();
    for claim in &claims {
        diagnostic = diagnostic.with_arg("fact", claim.as_str());
    }
    diagnostic.with_help(format!("known here: {}", claims.join("; ")))
}

/// The message of a failure: the checker's text naming the culprit if it has one, else the
/// general description of the kind of error.
pub fn failure_message(failure: &SourceCheckFailure) -> String {
    let (_, general) = failure_details(failure.error);
    failure.detail.clone().unwrap_or_else(|| general.to_owned())
}

/// The diagnostic for a failure. Its primary span is the exact expression being checked when
/// the checker knows it, else the enclosing construct; the construct is a related span when the
/// two differ, and the proved facts are arguments and help.
pub fn failure_diagnostic(failure: &SourceCheckFailure) -> Diagnostic {
    let (code, _) = failure_details(failure.error);
    let mut diagnostic = Diagnostic::new(code, failure_message(failure));
    match (failure.culprit, failure.span) {
        (Some(culprit), enclosing) => {
            diagnostic = diagnostic
                .at_span(culprit)
                .with_help_on_span("the expression being checked");
            if let Some(enclosing) = enclosing.filter(|enclosing| *enclosing != culprit) {
                diagnostic.related_mut().push(RelatedSpan {
                    span: enclosing,
                    label: "enclosing source construct".to_owned(),
                });
            }
        }
        (None, Some(enclosing)) => {
            diagnostic = diagnostic
                .at_span(enclosing)
                .with_help_on_span("enclosing source construct");
        }
        (None, None) => {}
    }
    with_facts(diagnostic, &failure.facts)
}

/// The diagnostic for a note (a warning), with the facts the checker held at that point.
pub fn note_diagnostic(note: &SourceCheckNote) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(note_code(note.kind), note.detail.as_str());
    if let Some(span) = note.span {
        diagnostic = diagnostic.at_span(span);
    }
    with_facts(diagnostic, &note.facts)
}

/// The enclosing construct of a failure, which is not always the primary span.
pub fn enclosing_span(failure: &SourceCheckFailure) -> Option<Span> {
    failure.span
}
