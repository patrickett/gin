use super::*;
use std::time::Duration;
use test_fixtures::TempPackage;

fn run(session: &mut InterpreterSession, expression: &str) -> TaskResult {
    let Submission::Command(Command::TaskStarted(id)) =
        session.submit(&format!("#run {expression}"))
    else {
        panic!("task did not start");
    };
    session.running_tasks[&id]
        .receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("task should finish without falling back to native execution")
}

fn assert_integer(submission: Submission, expected: i64) {
    let Submission::Evaluated { value, diagnostics } = submission else {
        panic!("expected evaluated integer, got {submission:?}");
    };
    assert_eq!(value.storage, RuntimeStorage::Int64(expected));
    assert!(!has_flaws(&diagnostics), "{diagnostics:?}");
}

fn assert_rejected(submission: Submission, code: &str, expression: &str) {
    let Submission::Rejected { diagnostics } = submission else {
        panic!("expected rejection, got {submission:?}");
    };
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.diagnostic().code.slug() == code)
        .unwrap_or_else(|| panic!("missing {code}: {diagnostics:?}"));
    assert_eq!(diagnostic.source(), expression);
    assert_eq!(diagnostic.display_path(), "<interpreter>");
}

fn next_heap_address(session: &InterpreterSession) -> typecheck::Address {
    session
        .eval_state
        .lock()
        .unwrap()
        .heap
        .clone()
        .allocate(ast::ConstValue::ZERO)
}

#[test]
fn foreground_and_background_share_integer_execution() {
    let mut session = InterpreterSession::default();
    assert!(matches!(
        session.submit("count := 40 + 2"),
        Submission::DeclarationAccepted { .. }
    ));
    for expression in ["40 + 2", "count"] {
        assert_integer(session.submit(expression), 42);
        let result = run(&mut session, expression);
        assert_eq!(result.state, TaskState::Completed);
        assert_integer(result.submission, 42);
    }
}

#[test]
fn integer_results_outside_the_display_range_are_never_truncated() {
    let mut session = InterpreterSession::default();
    for expression in ["9223372036854775808", "18446744073709551615"] {
        let Submission::Evaluated { value, .. } = session.submit(expression) else {
            panic!("wide integers should evaluate");
        };
        assert_eq!(value.display(), expression);
        assert!(value.gin_type.is_some());
        let result = run(&mut session, expression);
        assert_eq!(result.state, TaskState::Completed);
        let Submission::Evaluated { value, .. } = result.submission else {
            panic!("wide integers should evaluate in a background task");
        };
        assert_eq!(value.display(), expression);
    }
}

#[test]
fn foreground_and_background_share_text_execution() {
    let mut session = InterpreterSession::default();
    let foreground = session.submit("(#target.triple)");
    let background = run(&mut session, "(#target.triple)");
    assert_eq!(background.state, TaskState::Completed);
    let Submission::Evaluated {
        value: foreground, ..
    } = foreground
    else {
        panic!("target triple should evaluate as text: {foreground:?}");
    };
    let Submission::Evaluated {
        value: background, ..
    } = background.submission
    else {
        panic!("target triple should evaluate as text: {background:?}");
    };
    assert_eq!(foreground, background);
    assert!(matches!(foreground.storage, RuntimeStorage::Bytes(_)));
}

#[test]
fn evaluator_limits_remain_recoverable_in_both_execution_modes() {
    let mut session = InterpreterSession::default();
    let declaration = format!(
        "{}spin(flag Word) Word:\n    while flag is 1 do\n        iteration: flag\n    return 42",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let accepted = session.submit(&declaration);
    assert!(
        matches!(accepted, Submission::DeclarationAccepted { .. }),
        "{accepted:?}"
    );
    let before = next_heap_address(&session);
    assert_rejected(
        session.submit("(spin(1))"),
        "compile-time-fuel-exhausted",
        "(spin(1))",
    );
    assert_eq!(next_heap_address(&session), before);
    assert_eq!(session.pending(), Some("(spin(1))"));
    assert_integer(session.submit("(spin(0))"), 42);
    let before = next_heap_address(&session);
    let result = run(&mut session, "(spin(1))");
    assert_eq!(result.state, TaskState::Failed);
    assert_rejected(
        result.submission,
        "compile-time-fuel-exhausted",
        "(spin(1))",
    );
    assert_eq!(next_heap_address(&session), before);
    assert_integer(run(&mut session, "(spin(0))").submission, 42);
}

#[test]
fn cancelled_tasks_preserve_cancellation_instead_of_failing_or_executing() {
    let context = InterpreterContext::isolated();
    let evaluation =
        SessionSource::default().evaluation_candidate_with_dispatches("40 + 2", 1, &HashMap::new());
    let result = run_task_submission(
        context,
        evaluation,
        TaskId(1),
        Arc::new(AtomicBool::new(true)),
        Arc::new(Mutex::new(typecheck::EvalState::default())),
    );
    assert_eq!(result.state, TaskState::Cancelled);
    let Submission::Cancelled { diagnostics } = result.submission else {
        panic!("cancelled task must retain its cancellation outcome");
    };
    assert_eq!(
        diagnostics[0].diagnostic().code.slug().as_str(),
        "compile-time-cancelled"
    );
    assert_eq!(diagnostics[0].source(), "40 + 2");
}

#[test]
fn native_emission_is_rejected_without_executing_a_fallback() {
    let package = TempPackage::new("interpreter_typed_execution");
    package.write_flask("interpreter_typed_execution");
    let context = InterpreterContext::load(package.path()).unwrap();
    let flask::CompileTarget::Concrete(target) = context.target() else {
        panic!("interpreter requires a concrete target");
    };
    package.write(
        "native/defs.gin",
        &test_fixtures::emission::increment_source(&target.raw),
    );
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);
    let imported = session.submit("use 'native' increment");
    assert!(
        matches!(imported, Submission::DeclarationAccepted { .. }),
        "{imported:?}"
    );
    assert_rejected(
        session.submit("(increment(41))"),
        "compile-time-runtime-dependency",
        "(increment(41))",
    );
    let result = run(&mut session, "(increment(41))");
    assert_eq!(result.state, TaskState::Failed);
    assert_rejected(
        result.submission,
        "compile-time-runtime-dependency",
        "(increment(41))",
    );
    assert_integer(session.submit("40 + 2"), 42);
}
