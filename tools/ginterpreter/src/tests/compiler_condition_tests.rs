use super::*;
use test_fixtures::TempPackage;

const BUILDER: &str = "build(flag Word):\n    retained Word: 7\n    while flag is 1 do\n        iteration: flag\n    return code\n";
const EMISSION: &str = "another(value Word) Word: return #emit(build(1), Inputs(value: value))\n";

fn session() -> InterpreterSession {
    let mut session = InterpreterSession::default();
    let flask::CompileTarget::Concrete(target) = session.context().target() else {
        panic!("concrete interpreter target");
    };
    let source = test_fixtures::emission::increment_source(&target.raw);
    for source in [&source, BUILDER] {
        let submission = session.submit(source);
        assert!(
            matches!(submission, Submission::DeclarationAccepted { .. }),
            "{submission:?}"
        );
    }
    session
}

fn assert_compiler_condition(condition: &InterpreterCondition) {
    let diagnostic = condition
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.diagnostic().code.slug() == "compile-time-fuel-exhausted")
        .expect("original evaluator trap");
    let frame = diagnostic
        .frames()
        .iter()
        .find(|frame| frame.function == "build")
        .unwrap();
    assert_eq!(
        frame.local("retained").unwrap().value,
        Ok(ast::ConstValue::Int(7.into()))
    );
    assert_eq!(frame.local("flag").unwrap().value, Ok(ast::ConstValue::ONE));
    assert!(!frame.source[frame.span.to_range()].is_empty());
    assert_eq!(condition.restarts(), &[Restart::Retry, Restart::Abort]);
    assert_eq!(
        diagnostic.frames().last().unwrap().function,
        "<compile-time expression>"
    );
}

#[test]
fn compiler_phase_conditions_preserve_the_image_and_retry_against_redefined_helpers() {
    let mut session = session();
    let before = session.declarations().to_string();
    let heap_before = session
        .eval_state
        .lock()
        .unwrap()
        .heap
        .clone()
        .allocate(ast::ConstValue::ZERO);
    let submission = session.submit(EMISSION);
    assert!(
        matches!(submission, Submission::Rejected { .. }),
        "{submission:?}"
    );
    assert_eq!(session.declarations(), before);
    assert_eq!(
        session
            .eval_state
            .lock()
            .unwrap()
            .heap
            .clone()
            .allocate(ast::ConstValue::ZERO),
        heap_before
    );
    assert_compiler_condition(session.condition().unwrap());
    let captured = session.condition().unwrap().clone();
    let query = session.submit("#frame 0");
    assert!(matches!(query, Submission::Answered { text } if text.contains("retained = 7")));
    let fixed = session.submit("build(flag Word): return code\n");
    assert!(
        matches!(fixed, Submission::DeclarationAccepted { .. }),
        "{fixed:?}"
    );
    assert_compiler_condition(&captured);
    let retry = session.retry().unwrap();
    assert!(
        matches!(retry, Submission::DeclarationAccepted { .. }),
        "{retry:?}"
    );
    assert!(session.condition().is_none());
    assert!(
        session
            .materialize()
            .unwrap()
            .contains("another(value Word)")
    );
}

#[test]
fn imported_compiler_frames_retain_their_package_source() {
    let package = TempPackage::new("interpreter_compile_time_frames");
    package.write_flask("interpreter_compile_time_frames");
    let session = session();
    let source = session.declarations().to_string();
    package.write("builders/defs.gin", &source);
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);
    let imported = session.submit("use 'builders' Word, Inputs, build\n");
    assert!(
        matches!(imported, Submission::DeclarationAccepted { .. }),
        "{imported:?}"
    );
    let failure = session.submit(EMISSION);
    assert!(
        matches!(failure, Submission::Rejected { .. }),
        "{failure:?}"
    );
    assert_compiler_condition(session.condition().unwrap());
    let frame = session
        .condition()
        .unwrap()
        .frames()
        .iter()
        .find(|frame| frame.function == "build")
        .unwrap();
    assert!(frame.display_path.ends_with("builders/defs.gin"));
    assert_eq!(frame.source.as_ref(), source);
    assert!(session.abort());
}

#[test]
fn speculative_inlining_defers_traps_until_execution_and_preserves_restarts() {
    let mut session = InterpreterSession::default();
    for source in [
        test_fixtures::preludes::WORD_PRELUDE,
        concat!(
            "#inline\nbuild(flag Word) Word:\n",
            "    retained Word: 7\n    while flag is 1 do\n        iteration: flag\n    return retained\n",
        ),
    ] {
        let result = session.submit(source);
        assert!(
            matches!(result, Submission::DeclarationAccepted { .. }),
            "{result:?}"
        );
    }
    let result = session.submit("answer Word: build(1)\n");
    assert!(
        matches!(result, Submission::DeclarationAccepted { .. }),
        "{result:?}"
    );
    let before = session.materialize().unwrap().to_owned();
    let result = session.submit("(answer)");
    assert!(matches!(result, Submission::Rejected { .. }), "{result:?}");
    assert_eq!(session.declarations(), before);
    let condition = session.condition().unwrap();
    let diagnostic = condition
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.diagnostic().code.slug() == "compile-time-fuel-exhausted")
        .unwrap_or_else(|| panic!("{condition:?}"));
    let frame = diagnostic
        .frames()
        .iter()
        .find(|frame| frame.function == "build")
        .unwrap();
    assert_eq!(
        frame.local("retained").unwrap().value,
        Ok(ast::ConstValue::Int(7.into()))
    );
    assert_eq!(frame.local("flag").unwrap().value, Ok(ast::ConstValue::ONE));
    assert_eq!(condition.restarts(), &[Restart::Retry, Restart::Abort]);
    let fixed = session.submit("#inline\nbuild(flag Word) Word: return flag\n");
    assert!(
        matches!(fixed, Submission::DeclarationAccepted { .. }),
        "{fixed:?}"
    );
    let retry = session.retry().unwrap();
    let Submission::Evaluated { value, .. } = retry else {
        panic!("{retry:?}");
    };
    assert_eq!(value.storage, RuntimeStorage::Int64(1));
    assert!(session.condition().is_none());
    assert!(session.materialize().unwrap().contains("answer Word"));
}
