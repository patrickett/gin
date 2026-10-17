use super::*;
use analysis::{PackageCache, SymbolKind};
use test_fixtures::TempPackage;

const SOURCE: &str = concat!(
    "-- café 🦀\nWord is in 0 to 255\n",
    "Word has\n    finish(self):\n        return\n",
    "count Word := 7\n",
    "--- Retains an unfinished result.\n",
    "echo(value Word) Word: return #todo\n",
);

fn accept(session: &mut InterpreterSession, input: &str) {
    let result = session.submit(input);
    assert!(
        matches!(result, Submission::DeclarationAccepted { .. }),
        "{result:?}"
    );
}

#[test]
fn scope_uses_individual_typed_symbols_from_multiline_submissions() {
    let mut session = InterpreterSession::default();
    accept(&mut session, SOURCE);
    let symbols = session.symbols();
    for (name, kind) in [
        ("Word", SymbolKind::Type),
        ("count", SymbolKind::Value),
        ("echo", SymbolKind::Function),
    ] {
        let symbol = symbols.iter().find(|symbol| symbol.name == name).unwrap();
        assert_eq!(symbol.kind, kind);
        assert!(symbol.ty.is_some(), "{symbol:?}");
        assert_eq!(&session.declarations()[symbol.span.to_range()], name);
    }
    let function = symbols.iter().find(|symbol| symbol.name == "echo").unwrap();
    assert_eq!(function.signature, "echo(value Word) Word");
    assert!(
        function
            .documentation
            .as_ref()
            .unwrap()
            .contains("unfinished")
    );
    assert!(session.scope().iter().any(|entry| entry.name == "Word"));
    assert!(session.materialize().is_err());
}

#[test]
fn queries_preserve_the_accepted_snapshot_and_the_pending_stack() {
    let mut session = InterpreterSession::default();
    accept(&mut session, SOURCE);
    let symbols = session.symbols();
    assert!(matches!(
        session.submit("(echo(1))"),
        Submission::Rejected { .. }
    ));
    let pending = session.pending().unwrap().to_owned();
    let frames = session.frames().to_vec();
    assert!(!frames.is_empty());
    let heap_before = session
        .eval_state
        .lock()
        .unwrap()
        .heap
        .clone()
        .allocate(ast::ConstValue::ZERO);
    for (expression, expected) in [("count", "Word"), ("echo(1)", "Word"), ("#todo", "#todo")] {
        assert!(
            matches!(session.type_of(expression), QueryOutcome::Answered(ty) if ty == expected)
        );
    }
    let byte = session.declarations().find("echo(").unwrap();
    assert!(session.hover_at(byte).unwrap().markdown.contains("echo"));
    assert_eq!(session.symbols(), symbols);
    assert_eq!(session.pending(), Some(pending.as_str()));
    assert_eq!(session.frames()[0].locals, frames[0].locals);
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
    let result = session.submit("broken Word: missing\n");
    assert!(matches!(result, Submission::Rejected { .. }));
    assert_eq!(session.symbols(), symbols);
    accept(&mut session, "echo(value Word) Word: return value\n");
    assert_eq!(
        session
            .symbols()
            .iter()
            .filter(|symbol| symbol.name == "echo")
            .count(),
        1
    );
    assert!(session.materialize().is_ok());
    session.submit("#clear");
    assert!(session.symbols().is_empty());
    assert!(session.hover_at(byte).is_none());
}

#[test]
fn interpreter_hover_matches_lsp_queries_and_checks_byte_boundaries() {
    let package = TempPackage::new("query_image");
    package.write_flask("query_image");
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);
    accept(&mut session, SOURCE);
    let source = session.declarations().to_owned();
    let path = package.write("image.gin", &source);
    let cache = PackageCache::for_test();
    cache.set_contents(&path, source.clone());
    for name in ["Word is", "count Word", "echo(value", "return"] {
        let byte = source.find(name).unwrap();
        assert_eq!(
            session.hover_at(byte),
            cache.hover(&path, byte as u32),
            "{name}"
        );
    }
    let inside_emoji = source.find('🦀').unwrap() + 1;
    assert!(session.hover_at(inside_emoji).is_none());
    assert!(session.hover_at(source.len() + 1).is_none());
    assert!(session.hover_at(usize::MAX).is_none());
}

#[test]
fn imported_aliases_are_visible_without_exposing_private_package_symbols() {
    let package = TempPackage::new("query_aliases");
    package.write_flask("query_aliases");
    package.write(
        "values/defs.gin",
        &format!("{SOURCE}private secret Word: 3\n"),
    );
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);
    accept(&mut session, "use 'values' Word as Byte, count as total\n");
    let symbols = session.symbols();
    assert_eq!(
        symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect::<Vec<_>>(),
        ["Byte", "total"]
    );
    let result = session.type_of("total");
    assert!(matches!(&result, QueryOutcome::Answered(_)), "{result:?}");
    let alias_byte = session.declarations().find("total").unwrap();
    assert!(
        session
            .hover_at(alias_byte)
            .unwrap()
            .markdown
            .contains("count")
    );
    assert!(session.documentation("total").unwrap().contains("count"));
    assert!(session.documentation("secret").is_none());
    assert!(session.materialize().is_err());
    session.submit("#reset");
    assert!(session.symbols().is_empty());
}

#[test]
fn type_query_diagnostics_refer_to_the_expression_without_generated_names() {
    let mut session = InterpreterSession::default();
    accept(&mut session, SOURCE);
    let expression = "missing(1)";
    let QueryOutcome::Rejected(diagnostics) = session.type_of(expression) else {
        panic!("unknown symbol must reject the query");
    };
    for diagnostic in diagnostics {
        assert!(!diagnostic.diagnostic().message.contains("__gin"));
        if diagnostic.diagnostic().code.slug() == "type-unknown-symbol" {
            assert_eq!(diagnostic.source(), expression);
            assert_eq!(diagnostic.display_path(), "<interpreter>");
            assert_eq!(
                &expression[diagnostic.diagnostic().span.to_range()],
                "missing"
            );
        }
    }
    assert!(session.pending().is_none());
}

#[test]
fn command_help_and_symbol_documentation_are_available_without_a_terminal() {
    let mut session = InterpreterSession::default();
    accept(&mut session, SOURCE);
    let Submission::Answered { text } = session.submit("#help") else {
        panic!("help must be available through the session API");
    };
    assert!(text.contains("#type <expression>"), "{text}");
    assert!(text.contains("#save <path>"), "{text}");
    assert!(text.contains("#doc <symbol>"), "{text}");
    assert!(text.contains("compile-time-known inputs"), "{text}");
    let Submission::Answered { text: queries } = session.submit("#queries") else {
        panic!("queries must be discoverable");
    };
    assert!(
        matches!(session.submit("#help queries"), Submission::Answered { text }
        if text == queries)
    );
    let Submission::Answered { text } = session.submit("#doc echo") else {
        panic!("accepted symbols must be inspectable");
    };
    assert!(text.contains("echo(value Word) Word"), "{text}");
    assert!(text.contains("Retains an unfinished result."), "{text}");
    assert!(session.documentation("missing").is_none());
}

#[test]
fn help_and_documentation_preserve_pending_work_and_accept_whitespace() {
    let mut session = InterpreterSession::default();
    accept(&mut session, SOURCE);
    assert!(matches!(
        session.submit("missing + 1"),
        Submission::Rejected { .. }
    ));
    let source = session.declarations().to_owned();
    let pending = session.pending().unwrap().to_owned();
    for command in [
        " #help\t queries ",
        "#queries",
        "#type\t count",
        "#doc\t  echo",
        "#doc missing",
        "#help missing",
        "#frame\t 0",
    ] {
        let result = session.submit(command);
        assert!(
            matches!(result, Submission::Answered { .. }),
            "{command}: {result:?}"
        );
        assert_eq!(session.declarations(), source);
        assert_eq!(session.pending(), Some(pending.as_str()));
    }
    assert!(
        matches!(session.submit("#type\t count"), Submission::Answered { text }
        if text == "Word")
    );
    assert!(
        matches!(session.submit("#doc\t"), Submission::Answered { text }
        if text == "usage: #doc <symbol>")
    );
}

#[test]
fn failed_type_commands_preserve_the_original_condition_for_retry() {
    let mut session = InterpreterSession::default();
    assert!(matches!(
        session.submit("#type missing"),
        Submission::Rejected { .. }
    ));
    assert!(session.pending().is_none());
    accept(
        &mut session,
        &format!(
            "{}echo(value Word) Word: return #todo\n",
            test_fixtures::preludes::WORD_PRELUDE
        ),
    );
    assert!(matches!(
        session.submit("echo(1)"),
        Submission::Rejected { .. }
    ));
    let pending = session.pending().unwrap().to_owned();
    let locals = session.frames()[0].locals.clone();
    let declarations = session.declarations().to_owned();

    assert!(matches!(
        session.submit("#type\t missing"),
        Submission::Rejected { .. }
    ));
    assert_eq!(session.pending(), Some(pending.as_str()));
    assert_eq!(session.frames()[0].locals, locals);
    assert_eq!(session.declarations(), declarations);

    accept(&mut session, "echo(value Word) Word: return value\n");
    let result = session.submit("#retry");
    assert!(
        matches!(&result, Submission::Evaluated { value, .. }
        if value.storage == RuntimeStorage::Int64(1)),
        "{result:?}"
    );
    assert!(session.pending().is_none());
}

#[test]
fn package_root_names_are_discoverable_before_submissions_and_after_clear() {
    let package = TempPackage::new("query_package_root");
    package.write_flask("query_package_root");
    let path = package.write(
        "definitions.gin",
        "--- The package answer.\nanswer := 42\nprivate hidden := 7\n",
    );
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);
    let location = format!("{}:2:1", path.canonicalize().unwrap().display());

    assert!(session.symbols().is_empty());
    assert!(session.declarations().is_empty());
    let scope = session.scope();
    let answer = scope.iter().find(|entry| entry.name == "answer").unwrap();
    assert!(answer.signature.contains(&location), "{scope:?}");
    assert!(scope.iter().any(|entry| entry.name == "hidden"));
    let documentation = session.documentation("answer").unwrap();
    assert!(
        documentation.contains("The package answer."),
        "{documentation}"
    );
    assert!(documentation.contains(&location), "{documentation}");
    assert!(session.pending().is_none());
    assert!(
        matches!(session.submit("(hidden)"), Submission::Evaluated { value, .. }
        if value.storage == RuntimeStorage::Int64(7))
    );

    accept(&mut session, "local := 1");
    session.submit("#clear");
    assert_eq!(session.scope(), scope);
    assert_eq!(session.documentation("answer"), Some(documentation));
    assert!(session.symbols().is_empty());
}

#[test]
fn package_scope_preserves_session_spans_and_prefers_local_aliases() {
    let package = TempPackage::new("query_package_shadow");
    package.write_flask("query_package_shadow");
    package.write("definitions.gin", "answer := 42\n");
    package.write(
        "nested/definitions.gin",
        "--- Selected answer.\nselected := 7\n",
    );
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);
    assert!(session.documentation("answer").is_some());
    accept(&mut session, "use 'nested' selected as answer");

    let symbols = session.symbols();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "answer");
    assert_eq!(
        &session.declarations()[symbols[0].span.to_range()],
        "selected as answer"
    );
    let scope = session.scope();
    assert_eq!(
        scope.iter().filter(|entry| entry.name == "answer").count(),
        1
    );
    assert_eq!(scope[0].signature, "use answer");
    let documentation = session.documentation("answer").unwrap();
    assert!(
        documentation.contains("Selected answer."),
        "{documentation}"
    );
    assert!(
        matches!(session.submit("(answer)"), Submission::Evaluated { value, .. }
        if value.storage == RuntimeStorage::Int64(7))
    );
}

#[test]
fn root_scope_lists_nested_names_only_after_explicit_session_imports() {
    let package = TempPackage::new("query_package_modules");
    package.write_flask("query_package_modules");
    package.write("definitions.gin", "root_value := 42\n");
    package.write(
        "nested/definitions.gin",
        "child_value := 7\nprivate child_secret := 9\n",
    );
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);

    for name in ["child_value", "child_secret"] {
        assert!(!session.scope().iter().any(|entry| entry.name == name));
        assert!(session.documentation(name).is_none());
    }
    accept(&mut session, "use 'nested' child_value");
    assert!(
        session
            .scope()
            .iter()
            .any(|entry| entry.name == "child_value")
    );
    assert!(session.documentation("child_value").is_some());
    assert!(session.documentation("child_secret").is_none());
    assert!(
        matches!(session.submit("(child_value)"), Submission::Evaluated { value, .. }
        if value.storage == RuntimeStorage::Int64(7))
    );
    assert!(matches!(
        session.submit("use 'nested' child_secret"),
        Submission::Rejected { .. }
    ));
}

#[test]
fn public_child_values_can_use_private_helpers_without_exporting_them() {
    let package = TempPackage::new("query_private_child_helper");
    package.write_flask("query_private_child_helper");
    package.write(
        "child/definitions.gin",
        "public_value: hidden\nprivate hidden := 7\n",
    );
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);

    accept(&mut session, "use 'child' public_value");
    let result = session.submit("(public_value)");
    assert!(
        matches!(&result, Submission::Evaluated { value, .. }
        if value.storage == RuntimeStorage::Int64(7)),
        "{result:?}"
    );
    assert!(session.documentation("hidden").is_none());
    assert!(matches!(
        session.submit("use 'child' hidden"),
        Submission::Rejected { .. }
    ));
}
