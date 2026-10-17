use super::*;
use crate::catalog::{CapabilityKind, capabilities, query_catalog};
use crate::{InterpreterContext, RuntimeStorage};
use test_fixtures::TempPackage;

fn accept(session: &mut InterpreterSession, source: &str) {
    let result = session.submit(source);
    assert!(
        matches!(result, Submission::DeclarationAccepted { .. }),
        "{result:?}"
    );
}

fn answer(session: &mut InterpreterSession, source: &str) -> String {
    let result = session.submit(source);
    let Submission::Answered { text } = result else {
        panic!("{source}: {result:?}");
    };
    text
}

#[test]
fn catalog_compiler_queries_match_parser_kinds_and_examples_execute() {
    fn query_name(query: ast::QueryKind) -> &'static str {
        match query {
            ast::QueryKind::Size => "size",
            ast::QueryKind::Alignment => "alignment",
            ast::QueryKind::IndexBits => "index_bits",
            ast::QueryKind::Target => "target",
        }
    }
    let queries = query_catalog(None);
    assert_eq!(queries.len(), 4);
    for entry in queries {
        let kind = entry
            .name
            .parse::<ast::QueryKind>()
            .expect("parser query name");
        assert_eq!(entry.name, query_name(kind));
        assert!(!entry.examples.is_empty(), "{}", entry.name);
        assert!(!entry.prerequisites.is_empty());
        assert!(!entry.result.is_empty());
        for example in entry.examples {
            let mut session = InterpreterSession::default();
            for setup in example.setup {
                accept(&mut session, setup);
            }
            let result = session.submit(example.source);
            assert!(
                matches!(result, Submission::Evaluated { .. }),
                "{}: {result:?}",
                example.source
            );
            assert!(session.pending().is_none());
        }
    }
    assert!(query_catalog(None).iter().all(|entry| entry.name != "type"));
    assert!(query_catalog(Some("domain")).is_empty());
    assert!(capabilities().iter().any(|entry| entry.name == "bits"
        && entry.kind == CapabilityKind::DeclarationAttribute));
    assert!(
        capabilities()
            .iter()
            .any(|entry| entry.name == "type" && entry.kind == CapabilityKind::SessionCommand)
    );
}

#[test]
fn focused_help_and_query_filters_share_the_catalog() {
    let mut session = InterpreterSession::default();
    let expected = answer(&mut session, "#help size");
    assert_eq!(expected, answer(&mut session, "#help #size"));
    for text in [expected, answer(&mut session, "#queries size")] {
        assert!(text.contains("#size(T)"), "{text}");
        assert!(text.contains("Argument T"), "{text}");
        assert!(text.contains("Prerequisites:"), "{text}");
        assert!(text.contains("Result:"), "{text}");
        assert!(text.contains("#size(@())"), "{text}");
    }
    assert_eq!(
        answer(&mut session, "#help queries"),
        answer(&mut session, "#queries")
    );
    assert!(answer(&mut session, "#help attributes").contains("#bits(width)"));
    assert!(answer(&mut session, "#help jsonl").contains("ginterpreter --jsonl --isolated"));
    assert!(answer(&mut session, "#queries not_a_query").contains("no matching"));
}

#[test]
fn search_matches_documentation_symbols_and_capabilities_without_mutation() {
    let mut session = InterpreterSession::default();
    accept(
        &mut session,
        "--- Counts café visits.\nvisits := 42\nunrelated := 7",
    );
    assert!(matches!(
        session.submit("missing"),
        Submission::Rejected { .. }
    ));
    let source = session.declarations().to_owned();
    let matches = session.search("CAFÉ");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "visits");
    assert!(
        session
            .search("#alignment")
            .iter()
            .any(|entry| entry.name == "#alignment" && entry.kind == "compiler_query")
    );
    for command in [
        "#search café",
        "#search nonexistent",
        "#complete vis",
        "#complete #si",
        "#queries size",
        "#help #size",
    ] {
        answer(&mut session, command);
        assert_eq!(session.declarations(), source);
        assert_eq!(session.pending(), Some("missing"));
    }
    assert_eq!(answer(&mut session, "#search"), "usage: #search <text>");
    assert_eq!(
        answer(&mut session, "#complete"),
        "usage: #complete <source>"
    );
    assert_eq!(session.pending(), Some("missing"));
}

#[test]
fn completion_uses_current_symbols_local_parameters_and_query_fields() {
    let mut session = InterpreterSession::default();
    accept(&mut session, "answer := 42");
    let completions = session.completions("ans");
    assert!(
        completions.iter().any(|entry| entry.label == "answer"),
        "{completions:?}"
    );
    assert!(
        completions
            .iter()
            .all(|entry| !entry.label.starts_with("__gin_"))
    );
    assert!(
        session
            .completions("#si")
            .iter()
            .any(|entry| entry.label == "size")
    );
    assert!(
        session
            .completions("#hel")
            .iter()
            .any(|entry| entry.label == "help")
    );
    assert_eq!(
        session
            .completions("#target.ar")
            .iter()
            .map(|entry| entry.label.as_str())
            .collect::<Vec<_>>(),
        ["arch"]
    );
    accept(&mut session, test_fixtures::preludes::WORD_PRELUDE);
    let completions = session.completions("compute(value Word) Word:\n    return val");
    assert!(
        completions.iter().any(|entry| entry.label == "value"),
        "{completions:?}"
    );
    assert!(!session.declarations().contains("compute"));
    assert!(
        matches!(session.submit("answer"), Submission::Evaluated { value, .. }
        if value.storage == RuntimeStorage::Int64(42))
    );
}

#[test]
fn package_search_and_completion_use_visible_names_and_import_documentation() {
    let package = TempPackage::new("interpreter_discovery");
    package.write_flask("interpreter_discovery");
    package.write("root.gin", "--- Root counter.\nroot_count := 42");
    package.write("nested/defs.gin", "--- Counts narwhals.\ncount := 7");
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut session = InterpreterSession::with_context(context);
    let root = session.search("Root counter");
    assert!(root.iter().any(|entry| {
        entry.name == "root_count"
            && entry
                .location
                .as_ref()
                .is_some_and(|path| path.contains("root.gin"))
    }));
    assert!(
        session
            .completions("root_c")
            .iter()
            .any(|entry| entry.label == "root_count")
    );
    assert!(session.search("narwhals").is_empty());
    accept(&mut session, "use 'nested' count as imported_count");
    assert!(
        session
            .search("narwhals")
            .iter()
            .any(|entry| entry.name == "imported_count")
    );
    assert!(
        session
            .completions("imported_c")
            .iter()
            .any(|entry| entry.label == "imported_count")
    );
}

#[test]
fn discovery_payloads_serialize_with_explicit_categories() {
    let queries = serde_json::to_value(query_catalog(Some("size"))).unwrap();
    assert_eq!(queries[0]["kind"], "compiler_query");
    assert_eq!(queries[0]["arguments"][0]["name"], "T");
    assert_eq!(queries[0]["examples"][0]["source"], "#size(@())");
    let mut session = InterpreterSession::default();
    accept(&mut session, "answer := 42");
    assert!(
        serde_json::to_value(session.search("answer"))
            .unwrap()
            .is_array()
    );
    assert!(
        serde_json::to_value(session.completions("ans"))
            .unwrap()
            .is_array()
    );
}
