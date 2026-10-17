use super::*;
use crate::{InterpreterSession, Submission};
use diagnostic::code::TypeCode;

fn accepted(session: &mut InterpreterSession, source: &str) {
    let result = session.submit(source);
    assert!(
        matches!(result, Submission::DeclarationAccepted { .. }),
        "{result:?}"
    );
}

fn unknown_symbol(session: &mut InterpreterSession, source: &str) -> InterpreterDiagnostic {
    let Submission::Rejected { diagnostics } = session.submit(source) else {
        panic!("expected an unknown-symbol rejection");
    };
    let diagnostic = diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.diagnostic().code.slug() == "type-unknown-symbol")
        .expect("unknown-symbol diagnostic");
    assert_eq!(diagnostic.source(), source);
    assert_eq!(diagnostic.display_path(), "<interpreter>");
    let span = diagnostic.diagnostic().span;
    assert_eq!(&source[span.start()..span.end()], "missing");
    assert_eq!(
        source[..span.start()]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count(),
        1
    );
    diagnostic
}

#[test]
fn declaration_errors_use_submitted_utf8_source_after_previous_declarations() {
    let mut session = InterpreterSession::default();
    accepted(&mut session, "--- précédent 🦀\nseed := 7");
    let retained = session.declarations().to_owned();
    unknown_symbol(&mut session, "--- café 🦀\nbroken: missing");
    assert_eq!(session.declarations(), retained);
}

#[test]
fn failed_redefinitions_rebase_after_removing_the_previous_declaration() {
    let mut session = InterpreterSession::default();
    accepted(&mut session, "seed := 7");
    accepted(&mut session, "answer := 1");
    let retained = session.declarations().to_owned();
    unknown_symbol(&mut session, "--- café 🦀\nanswer: missing");
    assert_eq!(session.declarations(), retained);
}

#[test]
fn nominal_operator_errors_name_types_and_explain_domain_projection() {
    let mut session = InterpreterSession::default();
    accepted(
        &mut session,
        "Quantity is in 1 to 10\nQuantity has finish(self): return",
    );
    accepted(
        &mut session,
        "Price is in 0 to 10000\nPrice has finish(self): return",
    );
    let source = "order_total(quantity Quantity, price Price):\n    return quantity * price";
    let Submission::Rejected { diagnostics } = session.submit(source) else {
        panic!("expected nominal arithmetic rejection");
    };
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.diagnostic().code.slug() == "type-nominal-operator-unavailable"
        })
        .expect("nominal operator diagnostic");
    assert_eq!(diagnostic.source(), source);
    assert_eq!(
        diagnostic.diagnostic().message,
        "operator `*` is not provided for `Quantity` and `Price`"
    );
    assert_eq!(diagnostic.diagnostic().arg("left_type"), Some("Quantity"));
    assert_eq!(diagnostic.diagnostic().arg("right_type"), Some("Price"));
    assert!(
        diagnostic
            .diagnostic()
            .help
            .as_deref()
            .unwrap()
            .contains("raw is in Quantity: value")
    );
    accepted(
        &mut session,
        concat!(
            "order_total(quantity Quantity, price Price):\n",
            "    q is in Quantity: quantity\n",
            "    p is in Price: price\n",
            "    return q * p",
        ),
    );
}

#[test]
fn invalid_record_field_rejects_the_expression_with_its_source_span() {
    let mut session = InterpreterSession::default();
    accepted(
        &mut session,
        "Parcel has\n    weight in 0 to 100\n    priority in 0 to 1\n    finish(self): return",
    );
    let retained = session.declarations().to_owned();
    let source = "Parcel(weight: 101, priority: 1)";
    let Submission::Rejected { diagnostics } = session.submit(source) else {
        panic!("out-of-domain constructor was accepted");
    };
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.diagnostic().code.slug() == "type-out-of-range")
        .expect("constructor domain diagnostic");
    assert_eq!(diagnostic.source(), source);
    let span = diagnostic.diagnostic().span;
    assert_eq!(&source[span.start()..span.end()], "101");
    assert_eq!(session.declarations(), retained);
    assert!(matches!(
        session.submit("Parcel(weight: 100, priority: 1)"),
        Submission::Evaluated { .. }
    ));
}

#[test]
fn related_spans_rebase_only_when_the_whole_diagnostic_fits_the_submission() {
    let previous = "seed := 7\n";
    let source = "answer: missing";
    let candidate = format!("{previous}{source}\n");
    let start = previous.len();
    let mut diagnostic = Diagnostic::new(TypeCode::Mismatch, "test")
        .at_span(Span::new(start + 8, start + source.len()));
    diagnostic.related_mut().push(diagnostic::RelatedSpan {
        span: Span::new(start, start + 6),
        label: "current declaration".into(),
    });
    let mut current = InterpreterDiagnostic::new(&candidate, "<interpreter>", diagnostic.clone());
    current.rebase_submission(&candidate, source, start);
    assert_eq!(current.source(), source);
    assert_eq!(current.diagnostic().span, Span::new(8, source.len()));
    assert_eq!(current.diagnostic().related()[0].span, Span::new(0, 6));

    diagnostic.related_mut()[0].span = Span::new(0, 4);
    let mut crossing = InterpreterDiagnostic::new(&candidate, "<interpreter>", diagnostic.clone());
    crossing.rebase_submission(&candidate, source, start);
    assert_eq!(crossing.source(), candidate);
    assert_eq!(crossing.diagnostic(), &diagnostic);

    let mut imported = InterpreterDiagnostic::new(&candidate, "dependency.gin", diagnostic.clone());
    imported.rebase_submission(&candidate, source, start);
    assert_eq!(imported.source(), candidate);
    assert_eq!(imported.diagnostic(), &diagnostic);
}

#[test]
fn frame_rebasing_preserves_previous_sources() {
    let previous = "seed := 7\n";
    let source = "answer: missing";
    let candidate = format!("{previous}{source}\n");
    let frame = crate::InterpreterFrame {
        function: "answer".into(),
        source: std::sync::Arc::from(candidate.as_str()),
        display_path: "<interpreter>".into(),
        span: Span::new(previous.len(), previous.len() + source.len()),
        locals: Vec::new(),
    };
    let mut earlier = frame.clone();
    earlier.span = Span::new(0, 4);
    let mut diagnostic = InterpreterDiagnostic::new(
        source,
        "<interpreter>",
        Diagnostic::new(TypeCode::Mismatch, "test"),
    )
    .with_frames(vec![frame, earlier]);
    diagnostic.rebase_submission(&candidate, source, previous.len());
    assert_eq!(diagnostic.frames()[0].source.as_ref(), source);
    assert_eq!(diagnostic.frames()[0].span, Span::new(0, source.len()));
    assert_eq!(diagnostic.frames()[1].source.as_ref(), candidate);
    assert_eq!(diagnostic.frames()[1].span, Span::new(0, 4));
}
