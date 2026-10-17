use super::*;
use test_fixtures::TempPackage;

const TYPES: &str = test_fixtures::preludes::WORD_PRELUDE;

fn accept(session: &mut InterpreterSession, source: &str) {
    let result = session.submit(source);
    assert!(
        matches!(result, Submission::DeclarationAccepted { .. }),
        "{result:?}"
    );
}

fn fixed_source(input: &str, action: &crate::CodeAction) -> String {
    let mut source = input.to_owned();
    let mut edits: Vec<_> = action.edits.iter().collect();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.span.start()));
    for edit in edits {
        source.replace_range(edit.span.to_range(), &edit.replacement);
    }
    source
}

#[test]
fn interpreter_and_formatter_offer_the_same_edits_in_submission_coordinates() {
    let mut session = InterpreterSession::default();
    accept(&mut session, TYPES);
    accept(&mut session, "previous Word:\n    return 1\n");
    let before = session.declarations().to_owned();
    let input = "-- café 🦀\nvalue Word:\n    7\n";
    let package = TempPackage::new("interpreter_code_actions");
    package.write_flask("interpreter_code_actions");
    let path = package.write("main.gin", &format!("{before}{input}"));
    let shared = ginfmt::help::suggestions(&path, session.context().target().clone()).unwrap();
    let actions = session.code_actions(input);
    assert_eq!(actions.len(), 1, "{actions:?}");
    let action = &actions[0];
    let expected = shared
        .iter()
        .find(|help| help.diagnostic.span.start() >= before.len())
        .unwrap();
    assert_eq!(action.diagnostic.code, expected.diagnostic.code);
    assert_eq!(
        action.diagnostic.span.start() + before.len(),
        expected.diagnostic.span.start()
    );
    assert_eq!(action.edits.len(), expected.edits.len());
    for (edit, expected) in action.edits.iter().zip(&expected.edits) {
        assert_eq!(edit.span.start() + before.len(), expected.span.start());
        assert_eq!(edit.span.end() + before.len(), expected.span.end());
        assert_eq!(edit.replacement, expected.replacement);
    }
    assert_eq!(session.declarations(), before);
    let fixed = fixed_source(input, action);
    assert_eq!(fixed, "-- café 🦀\nvalue Word: 7\n");
    accept(&mut session, &fixed);
    assert!(session.materialize().unwrap().contains("value Word: 7"));
}

#[test]
fn receiver_actions_resolve_imported_methods_and_shadowed_declarations() {
    let package = TempPackage::new("interpreter_imported_actions");
    package.write_flask("interpreter_imported_actions");
    let storage = format!(
        "{TYPES}{}",
        concat!(
            "Box has\n",
            "    value Word\n",
            "    read(self ref) Word: return self.value\n",
            "    finish(self):\n        return\n",
        )
    );
    package.write("storage/defs.gin", &storage);
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);
    accept(&mut session, "use 'storage' Word, Box\n");
    accept(&mut session, "read(box ref Box) Word: return 1\n");
    let before = session.declarations().to_owned();
    let input = "read(box ref Box) Word: return Box.read(box)\n";
    let actions = session.code_actions(input);
    let action = actions
        .iter()
        .find(|help| help.diagnostic.code.slug() == "fmt-receiver-call")
        .unwrap_or_else(|| panic!("{actions:?}"));
    assert_eq!(&input[action.edits[0].span.to_range()], "Box.read(box)");
    assert_eq!(action.edits[0].replacement, "box.read");
    assert_eq!(session.declarations(), before);
    accept(&mut session, &fixed_source(input, action));
    assert!(
        session
            .materialize()
            .unwrap()
            .contains("read(box ref Box) Word: return box.read")
    );
    let expression = "(Box.read(Box(value: 7)))";
    let actions = session.code_actions(expression);
    let action = actions
        .iter()
        .find(|help| help.diagnostic.code.slug() == "fmt-receiver-call")
        .unwrap_or_else(|| panic!("{actions:?}"));
    assert_eq!(fixed_source(expression, action), "(Box(value: 7).read)");
}

#[test]
fn suggesting_repairs_keeps_the_pending_condition_and_accepted_image() {
    let mut session = InterpreterSession::default();
    accept(&mut session, TYPES);
    let before = session.declarations().to_owned();
    let input = "value Word:\n    missing\n";
    assert!(matches!(session.submit(input), Submission::Rejected { .. }));
    let pending = session.pending().unwrap().to_owned();
    let result = session.submit("#suggest");
    let Submission::Answered { text } = result else {
        panic!("{result:?}");
    };
    assert!(text.contains("fmt-single-expression-body"), "{text}");
    assert_eq!(session.pending(), Some(pending.as_str()));
    assert_eq!(session.declarations(), before);
    let actions = session.code_actions(input);
    let fixed = fixed_source(input, &actions[0]);
    assert!(matches!(
        session.submit(&fixed),
        Submission::Rejected { .. }
    ));
    assert_eq!(session.declarations(), before);
    accept(&mut session, "missing Word: 7\n");
    assert!(matches!(
        session.retry(),
        Some(Submission::DeclarationAccepted { .. })
    ));
    assert!(session.condition().is_none());
}

#[test]
fn hole_actions_remain_queries_and_the_commit_gate_stays_strict() {
    let mut session = InterpreterSession::default();
    accept(&mut session, TYPES);
    accept(&mut session, "unfinished:\n    #todo\n");
    let before = session.declarations().to_owned();
    let result = session.submit("#suggest");
    let Submission::Answered { text } = result else {
        panic!("{result:?}");
    };
    assert!(text.contains("fmt-single-expression-body"), "{text}");
    assert_eq!(session.declarations(), before);
    assert!(session.materialize().is_err());
    assert!(matches!(
        session.submit("(unfinished)"),
        Submission::Rejected { .. }
    ));
    let frames = session.frames().to_vec();
    let _ = session.code_actions("replacement Word:\n    #todo\n");
    assert_eq!(session.frames().len(), frames.len());
    assert_eq!(session.frames()[0].function, frames[0].function);
    assert_eq!(session.frames()[0].locals, frames[0].locals);
    assert_eq!(session.pending(), Some("(unfinished)"));
}
