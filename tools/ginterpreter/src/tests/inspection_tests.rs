use super::*;
use crate::value::{InspectionLimits, ValueKind};

#[test]
fn inspection_preserves_declarations_pending_condition_and_heap() {
    let mut session = InterpreterSession::default();
    let source = concat!(
        "Word is in 0 to 255\nWord has\n    finish(self):\n        return\n",
        "outer:\n    value Word: 1\n    value.finish\n    value:: 7\n    return 42\n",
    );
    assert!(matches!(
        session.submit(source),
        Submission::DeclarationAccepted { .. }
    ));
    assert!(matches!(
        session.submit("missing + 1"),
        Submission::Rejected { .. }
    ));
    let pending = session.pending().unwrap().to_owned();
    let declarations = session.declarations().to_owned();
    let before = session.eval_state.lock().unwrap().clone();
    let before_address = before.heap.clone().allocate(ast::ConstValue::ZERO);
    let generation = session.next_generation;
    let Submission::Inspected { value, .. } = session.inspect("outer") else {
        panic!("inspection should evaluate the function");
    };
    assert_eq!(value.display(), "42");
    assert_eq!(session.declarations(), declarations);
    assert_eq!(session.pending(), Some(pending.as_str()));
    assert_eq!(session.next_generation, generation);
    let after = session.eval_state.lock().unwrap();
    assert_eq!(
        after.heap.clone().allocate(ast::ConstValue::ZERO),
        before_address
    );
    assert_eq!(after.fuel, before.fuel);
    drop(after);
    assert!(matches!(
        session.submit("outer"),
        Submission::Evaluated { .. }
    ));
    assert_ne!(session.eval_state.lock().unwrap().fuel, before.fuel);
}

#[test]
fn inspection_returns_typed_propositions_and_preserves_prior_failures() {
    let mut session = InterpreterSession::default();
    assert!(matches!(
        session.submit("missing + 1"),
        Submission::Rejected { .. }
    ));
    let Submission::Inspected { value, .. } = session.inspect("1 < 2") else {
        panic!("inspection should return the proposition");
    };
    assert!(matches!(value.gin_type, Some(ast::ty::Ty::Proposition(_))));
    let inspection = value.inspection(InspectionLimits::default());
    assert_eq!(inspection.nodes[0].kind, ValueKind::Proposition);
    assert_eq!(inspection.nodes[0].value.as_deref(), Some("true"));
    assert_eq!(inspection.display, "holds");
    assert!(matches!(
        session.inspect("other_missing + 1"),
        Submission::Rejected { .. }
    ));
    assert_eq!(session.pending(), Some("missing + 1"));
}

#[test]
fn bare_return_reports_completion_without_fabricating_a_value_syntax() {
    let mut session = InterpreterSession::default();
    assert!(matches!(
        session.submit("finish:\n    return"),
        Submission::DeclarationAccepted { .. }
    ));
    let Submission::Evaluated { value, .. } = session.submit("finish") else {
        panic!("bare-return functions should evaluate");
    };
    assert_eq!(value.gin_type, Some(ast::ty::Ty::Unit));
    assert_eq!(
        value.inspection(InspectionLimits::default()).nodes[0].kind,
        ValueKind::Unit
    );
    assert_eq!(value.display(), "completed (no value)");
}
