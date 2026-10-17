use super::*;
use crate::{InterpreterCondition, InterpreterFrame};
use std::time::{Duration, Instant};
use test_fixtures::TempPackage;

const TYPES: &str = test_fixtures::preludes::WORD_PRELUDE;
const INNER: &str = "inner(flag Word) Word:\n    local Word: 7\n    while flag is 1 do\n        iteration: flag\n    return local\n";
const OUTER: &str = "outer(flag Word) Word:\n    retained Word: 9\n    return inner(flag)\n";

fn session() -> InterpreterSession {
    let mut session = InterpreterSession::default();
    for source in [TYPES, INNER, OUTER] {
        let accepted = session.submit(source);
        assert!(
            matches!(accepted, Submission::DeclarationAccepted { .. }),
            "{accepted:?}"
        );
    }
    session
}

fn assert_local(frame: &InterpreterFrame, name: &str, expected: i128) {
    let local = frame.local(name).unwrap();
    assert_eq!(local.value, Ok(ast::ConstValue::Int(expected.into())));
}

fn assert_frames(condition: &InterpreterCondition) {
    assert_eq!(
        condition
            .frames()
            .iter()
            .map(|frame| frame.function.as_str())
            .collect::<Vec<_>>(),
        ["inner", "outer", "<expression>"]
    );
    assert_local(condition.frame(0).unwrap(), "local", 7);
    assert_local(condition.frame(1).unwrap(), "retained", 9);
    assert_eq!(condition.frame(2).unwrap().source.as_ref(), "(outer(1))");
    assert!(condition.frame(3).is_none());
    assert_eq!(condition.restarts(), &[Restart::Retry, Restart::Abort]);
    assert!(Arc::ptr_eq(
        &condition.frame(0).unwrap().source,
        &condition.frame(1).unwrap().source
    ));
}

fn answer(submission: Submission) -> String {
    let Submission::Answered { text } = submission else {
        panic!("expected query answer: {submission:?}")
    };
    text
}

fn poll(session: &mut InterpreterSession) -> TaskResult {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(result) = session.poll_task_results().pop() {
            return result;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("background evaluation timed out");
}

#[test]
fn trapped_conditions_keep_frames_through_queries_edits_and_failed_retries() {
    let mut session = session();
    let result = session.submit("(outer(1))");
    assert!(matches!(result, Submission::Rejected { .. }));
    assert_frames(session.condition().unwrap());
    let retained = session.condition().unwrap().clone();
    let frames = answer(session.submit("#frames"));
    assert!(frames.starts_with("0: inner at <interpreter>:"), "{frames}");
    assert!(frames.contains("2: <expression>"), "{frames}");
    assert!(!frames.contains("__gin"));
    let frame = answer(session.submit("#frame 0"));
    assert!(frame.contains("local = 7"), "{frame}");
    assert!(frame.contains("flag = 1"), "{frame}");
    assert_eq!(answer(session.submit("#frame 99")), "unknown frame index");
    assert!(matches!(session.retry(), Some(Submission::Rejected { .. })));
    assert_frames(session.condition().unwrap());
    assert_frames(&retained);
    assert!(matches!(
        session.submit("inner(flag Word) Word: return 42"),
        Submission::DeclarationAccepted { .. }
    ));
    assert_frames(session.condition().unwrap());
    let retried = session.submit("#retry");
    assert!(
        matches!(&retried, Submission::Evaluated { value, .. }
        if value.storage == RuntimeStorage::Int64(42)),
        "{retried:?}"
    );
    assert!(session.condition().is_none());
    assert!(session.frames().is_empty());
    assert!(session.restarts().is_empty());
}

#[test]
fn abort_and_reset_release_conditions_and_frame_queries_leave_them_unchanged() {
    let mut session = session();
    session.submit("(outer(1))");
    for command in [
        "#frame",
        "#frame -1",
        "#frames invalid",
        "#frames 1 2",
        "#retry invalid",
    ] {
        assert!(answer(session.submit(command)).starts_with("usage:"));
        assert_frames(session.condition().unwrap());
    }
    assert_eq!(
        answer(session.submit("#abort")),
        "pending condition discarded"
    );
    assert!(session.frame(0).is_none());
    assert_eq!(answer(session.submit("#frames")), "no pending condition");
    session.submit("(outer(1))");
    session.submit("#reset");
    assert!(session.condition().is_none());
    session.submit("missing");
    assert!(session.condition().is_some());
    assert_eq!(
        answer(session.submit("#frames")),
        "condition has no evaluator frames"
    );
}

#[test]
fn background_conditions_have_independent_queries_and_restarts() {
    let mut session = session();
    session.submit("(outer(1))");
    let Submission::Command(Command::TaskStarted(id)) = session.submit("#run (outer(1))") else {
        panic!("task should start");
    };
    let failed = poll(&mut session);
    assert_eq!(failed.id, id);
    assert_eq!(failed.state, TaskState::Failed);
    assert_frames(session.task_condition(id).unwrap());
    assert_frames(session.condition().unwrap());
    assert!(answer(session.submit(&format!("#frame 0 {}", id.0))).contains("local = 7"));
    assert!(matches!(
        session.submit("inner(flag Word) Word: return 42"),
        Submission::DeclarationAccepted { .. }
    ));
    let Submission::Command(Command::TaskStarted(retried)) =
        session.submit(&format!("#retry {}", id.0))
    else {
        panic!("retry should start a new task");
    };
    assert_ne!(retried, id);
    assert!(session.task_condition(id).is_none());
    assert_frames(session.condition().unwrap());
    let completed = poll(&mut session);
    assert_eq!(completed.state, TaskState::Completed, "{completed:?}");
    assert!(session.abort());
    assert!(session.condition().is_none());
}

#[test]
fn imported_frames_retain_package_source_and_locations() {
    let package = TempPackage::new("interpreter_frame_source");
    package.write_flask("interpreter_frame_source");
    let declarations = [TYPES, INNER, OUTER].concat();
    package.write("nested/defs.gin", &declarations);
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);
    let imported = session.submit("use 'nested' Word, inner, outer");
    assert!(
        matches!(imported, Submission::DeclarationAccepted { .. }),
        "{imported:?}"
    );
    session.submit("(outer(1))");
    let frame = session.frame(0).unwrap();
    assert_eq!(frame.function, "inner");
    assert_eq!(frame.source.as_ref(), declarations);
    assert!(
        frame.display_path.ends_with("nested/defs.gin"),
        "{}",
        frame.display_path
    );
    assert!(
        frame
            .source
            .get(frame.span.to_range())
            .is_some_and(|source| !source.is_empty())
    );
    assert_local(frame, "local", 7);
}

#[test]
fn background_abort_releases_only_the_selected_condition_and_reset_clears_the_rest() {
    let mut session = session();
    session.submit("(outer(1))");
    let mut ids = Vec::new();
    for _ in 0..2 {
        let Submission::Command(Command::TaskStarted(id)) = session.submit("#run (outer(1))")
        else {
            panic!("task should start");
        };
        assert_eq!(poll(&mut session).state, TaskState::Failed);
        ids.push(id);
    }
    assert_eq!(
        answer(session.submit(&format!("#abort {}", ids[0].0))),
        "pending condition discarded"
    );
    assert!(session.task_condition(ids[0]).is_none());
    assert_frames(session.task_condition(ids[1]).unwrap());
    assert_frames(session.condition().unwrap());
    assert!(answer(session.submit(&format!("#frames {}", ids[1].0))).contains("0: inner"));
    session.submit("#reset");
    assert!(session.task_condition(ids[1]).is_none());
    assert!(session.condition().is_none());
}
