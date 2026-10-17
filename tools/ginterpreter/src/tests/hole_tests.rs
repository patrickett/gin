use super::*;
use diagnostic::Category;
use test_fixtures::TempPackage;

const TYPES: &str = test_fixtures::preludes::WORD_PRELUDE;

fn session() -> InterpreterSession {
    let mut session = InterpreterSession::default();
    accept(&mut session, TYPES);
    session
}

fn accept(session: &mut InterpreterSession, source: &str) -> Vec<InterpreterDiagnostic> {
    let submission = session.submit(source);
    let Submission::DeclarationAccepted { diagnostics } = submission else {
        panic!("expected accepted declaration: {submission:?}");
    };
    assert!(!has_flaws(&diagnostics), "{diagnostics:?}");
    diagnostics
}

fn assert_integer(submission: Submission, expected: i64) {
    let Submission::Evaluated { value, .. } = submission else {
        panic!("expected evaluated integer: {submission:?}");
    };
    assert_eq!(value.storage, RuntimeStorage::Int64(expected));
}

fn assert_hole_gate(session: &mut InterpreterSession, expected: usize) {
    let diagnostics = session.materialize().unwrap_err();
    let holes = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.diagnostic().code.slug() == "unfilled-hole")
        .collect::<Vec<_>>();
    assert_eq!(holes.len(), expected, "{diagnostics:?}");
    for hole in holes {
        assert_eq!(hole.diagnostic().category, Category::Flaw);
        assert_eq!(&hole.source()[hole.diagnostic().span.to_range()], "#todo");
    }
}

#[test]
fn holes_warn_remain_queryable_and_retry_after_redefinition() {
    let mut session = session();
    let warnings = accept(
        &mut session,
        "unfinished Word:\n    retained Word: 7\n    return #todo\n",
    );
    assert!(
        warnings.iter().any(
            |diagnostic| diagnostic.diagnostic().code.slug() == "unfilled-hole"
                && diagnostic.diagnostic().category == Category::Warn
        )
    );
    assert!(matches!(session.type_of("unfinished"), QueryOutcome::Answered(ty) if ty == "Word"));
    assert_hole_gate(&mut session, 1);
    let before = session
        .eval_state
        .lock()
        .unwrap()
        .heap
        .clone()
        .allocate(ast::ConstValue::ZERO);
    assert!(matches!(
        session.submit("(unfinished)"),
        Submission::Rejected { .. }
    ));
    let frame = session.frame(0).unwrap();
    assert_eq!(frame.function, "unfinished");
    assert_eq!(
        frame.local("retained").unwrap().value,
        Ok(ast::ConstValue::Int(7.into()))
    );
    assert_eq!(
        session
            .condition()
            .unwrap()
            .diagnostics
            .last()
            .unwrap()
            .diagnostic()
            .code
            .slug()
            .as_str(),
        "compile-time-unfilled-hole"
    );
    assert_eq!(
        session
            .eval_state
            .lock()
            .unwrap()
            .heap
            .clone()
            .allocate(ast::ConstValue::ZERO),
        before
    );
    accept(&mut session, "unfinished Word: 42\n");
    assert_integer(session.retry().unwrap(), 42);
    assert!(session.condition().is_none());
    assert!(!session.materialize().unwrap().contains("#todo"));
}

#[test]
fn unused_and_constant_unselected_holes_still_block_materialization() {
    let mut session = session();
    accept(&mut session, "unused Word: #todo\n");
    let arch = session.context().target().arch_literal().unwrap();
    accept(
        &mut session,
        &format!("selected Word := when #target.arch is '{arch}' then 42 else #todo\n"),
    );
    assert_integer(session.submit("(selected)"), 42);
    assert_hole_gate(&mut session, 2);
    accept(&mut session, "unused Word: 7\n");
    assert_hole_gate(&mut session, 1);
    accept(&mut session, "selected Word := 42\n");
    let source = session.materialize().unwrap().to_string();
    let mut replay = InterpreterSession::default();
    accept(&mut replay, &source);
    assert_integer(replay.submit("(selected)"), 42);
}

#[test]
fn save_checks_holes_before_creating_or_overwriting_a_file() {
    let package = TempPackage::new("interpreter_hole_save");
    let existing = package.write("saved.gin", "preserved contents");
    let missing = package.path().join("new.gin");
    let mut session = session();
    accept(&mut session, "unfinished Word: #todo\n");
    for path in [&existing, &missing] {
        let saved = session.submit(&format!("#save {}", path.display()));
        assert!(matches!(saved, Submission::Rejected { .. }), "{saved:?}");
    }
    assert_eq!(
        std::fs::read_to_string(&existing).unwrap(),
        "preserved contents"
    );
    assert!(!missing.exists());
    accept(&mut session, "unfinished Word: 42\n");
    let saved = session.submit(&format!("#save {}", existing.display()));
    assert!(matches!(saved, Submission::Answered { .. }), "{saved:?}");
    assert_eq!(
        std::fs::read_to_string(existing).unwrap(),
        session.materialize().unwrap()
    );
}

#[test]
fn imported_holes_keep_source_locations_and_block_the_commit_gate() {
    let package = TempPackage::new("interpreter_imported_hole");
    package.write_flask("interpreter_imported_hole");
    let source = format!("{TYPES}unfinished Word: #todo\n");
    package.write("unfinished/defs.gin", &source);
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);
    accept(&mut session, "use 'unfinished' Word, unfinished\n");
    let errors = session.materialize().unwrap_err();
    let hole = errors
        .iter()
        .find(|diagnostic| diagnostic.diagnostic().code.slug() == "unfilled-hole")
        .unwrap();
    assert!(hole.display_path().ends_with("unfinished/defs.gin"));
    assert_eq!(hole.source(), source);
    assert!(matches!(
        session.submit("(unfinished)"),
        Submission::Rejected { .. }
    ));
    assert!(
        session
            .frame(0)
            .unwrap()
            .display_path
            .ends_with("unfinished/defs.gin")
    );
}

#[test]
fn materialization_rechecks_imports_changed_after_image_acceptance() {
    for external in [false, true] {
        let package = TempPackage::new("interpreter_changed_import_hole");
        package.write_flask_with_deps(
            "interpreter_changed_import_hole",
            r#"changed: "./dependency"
"#,
        );
        package.write_nested_flask("dependency", "changed");
        let complete = format!("{TYPES}value Word: 42\n");
        let (relative, import) = if external {
            (
                "dependency/values/defs.gin",
                "use changed.values Word, value\n",
            )
        } else {
            ("values/defs.gin", "use 'values' Word, value\n")
        };
        let path = package.write(relative, &complete);
        let context = InterpreterContext::load(package.path()).unwrap();
        let mut session = InterpreterSession::with_context(context);
        accept(&mut session, import);
        let before = session.materialize().unwrap().to_owned();
        let symbols = session.symbols();
        let unfinished = format!("{complete}unused Word: #todo\n");
        std::fs::write(&path, &unfinished).unwrap();
        assert_hole_gate(&mut session, 1);
        assert_eq!(session.declarations(), before);
        assert_eq!(session.symbols(), symbols);
        let destination = TempPackage::new("interpreter_changed_import_save");
        let saved = destination.write("saved.gin", "preserved contents");
        let result = session.submit(&format!("#save {}", saved.display()));
        assert!(matches!(result, Submission::Rejected { .. }), "{result:?}");
        assert_eq!(
            std::fs::read_to_string(&saved).unwrap(),
            "preserved contents"
        );
        std::fs::remove_file(&path).unwrap();
        assert!(session.materialize().is_err());
        std::fs::write(&path, complete).unwrap();
        assert_eq!(session.materialize().unwrap(), before);
        let added = package.write("added.gin", "#todo\n");
        assert_hole_gate(&mut session, 1);
        std::fs::remove_file(added).unwrap();
        assert_eq!(session.materialize().unwrap(), before);
        assert_integer(session.submit("(value)"), 42);
    }
}

#[test]
fn materialization_uses_current_dependency_configuration_without_replacing_the_image() {
    let package = TempPackage::new("interpreter_changed_dependency_config");
    let name = "interpreter_changed_dependency_config";
    let original = r#"changed: "./original"
"#;
    let replacement = r#"changed: "./replacement"
"#;
    package.write_flask_with_deps(name, original);
    for directory in ["original", "replacement"] {
        package.write_nested_flask(directory, "changed");
        package.write(
            &format!("{directory}/values/defs.gin"),
            &format!("{TYPES}value Word: 42\n"),
        );
    }
    package.write(
        "replacement/values/defs.gin",
        &format!("{TYPES}value Word: #todo\n"),
    );
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);
    accept(&mut session, "use changed.values Word, value\n");
    let before = session.materialize().unwrap().to_owned();
    let symbols = session.symbols();
    package.write_flask_with_deps(name, replacement);
    assert_hole_gate(&mut session, 1);
    assert_eq!(session.declarations(), before);
    assert_eq!(session.symbols(), symbols);
    assert!(session.pending().is_none());
    package.write_flask_with_deps(name, original);
    assert_eq!(session.materialize().unwrap(), before);
    assert_integer(session.submit("(value)"), 42);
}

#[test]
fn holes_do_not_mask_errors_or_persist_failed_expressions() {
    let mut session = session();
    let before = session.declarations().to_string();
    let bad = session.submit("unfinished Word: #todo\nbroken Word: 256\n");
    assert!(matches!(bad, Submission::Rejected { .. }), "{bad:?}");
    assert_eq!(session.declarations(), before);
    assert!(matches!(
        session.submit("#todo"),
        Submission::Rejected { .. }
    ));
    assert!(session.frame(0).is_some());
    assert_eq!(session.materialize().unwrap(), before);
    assert!(session.abort());
    accept(&mut session, "todo Word := 42\n-- #todo\n");
    assert_integer(session.submit("todo"), 42);
    assert!(session.materialize().is_ok());
}

#[test]
fn background_hole_conditions_offer_retry_and_abort() {
    let mut session = session();
    accept(&mut session, "unfinished Word: #todo\n");
    let id = session.spawn_task("(unfinished)");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if !session.poll_task_results().is_empty() {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(1));
    }
    let condition = session
        .task_condition(id)
        .expect("background hole condition");
    assert_eq!(condition.frame(0).unwrap().function, "unfinished");
    assert_eq!(condition.restarts(), &[Restart::Retry, Restart::Abort]);
    assert!(session.abort_task(id));
    assert!(session.task_condition(id).is_none());
}
