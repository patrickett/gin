use super::*;

#[test]
fn candidate_includes_prior_declarations_without_mutating_them() {
    let mut source = SessionSource::default();
    source.accept_declaration("first := 1", vec!["first".to_string()]);

    assert_eq!(source.candidate("first + 1"), "first := 1\nfirst + 1\n");
    assert_eq!(source.declarations(), "first := 1\n");
}

#[test]
fn accepted_declarations_accumulate_in_order() {
    let mut source = SessionSource::default();

    source.accept_declaration("first := 1", vec!["first".to_string()]);
    source.accept_declaration("second := first", vec!["second".to_string()]);

    assert_eq!(source.declarations(), "first := 1\nsecond := first\n");
}

#[test]
fn accepted_declaration_replaces_previous_logical_definition() {
    let mut source = SessionSource::default();

    source.accept_declaration("value Signed64 := 1", vec!["value".to_string()]);
    let first_slot = source.symbol_slot("value").expect("slot created");
    source.accept_declaration("value Signed64 := 2", vec!["value".to_string()]);

    assert_eq!(source.declarations(), "value Signed64 := 2\n");
    assert_eq!(
        source.symbol_slot("value"),
        Some(first_slot),
        "slot must be stable across logical redeclarations",
    );
}

#[test]
fn clear_discards_declarations() {
    let mut source = SessionSource::default();
    source.accept_declaration("first := 1", vec!["first".to_string()]);

    source.clear();

    assert!(source.declarations().is_empty());
}

#[test]
fn candidate_with_overlapping_symbols_skips_shadowed_declarations() {
    let mut source = SessionSource::default();
    source.accept_declaration("value Signed64 := 1", vec!["value".to_string()]);
    source.accept_declaration("other := value", vec!["other".to_string()]);

    let candidate =
        source.candidate_without_shadowed("value Signed64 := 3", &["value".to_string()]);

    assert_eq!(candidate, "other := value\nvalue Signed64 := 3\n");
}

#[test]
fn replacing_one_name_preserves_siblings_imports_and_their_metadata() {
    let mut source = SessionSource::default();
    let original = concat!(
        "use 'numbers' Word\n",
        "-- café 🦀\n",
        "first Word := 1\n",
        "--- The other value.\n",
        "second Word := 2",
    );
    source.accept_declaration(original, vec!["first".into(), "second".into()]);
    let second_slot = source.symbol_slot("second");
    let expected = concat!(
        "use 'numbers' Word\n",
        "-- café 🦀\n",
        "--- The other value.\n",
        "second Word := 2\n",
        "first Word := 3\n",
    );

    assert_eq!(
        source.candidate_without_shadowed("first Word := 3", &["first".into()]),
        expected
    );
    assert_eq!(source.declarations(), format!("{original}\n"));
    source.accept_declaration("first Word := 3", vec!["first".into()]);
    assert_eq!(source.declarations(), expected);
    assert_eq!(source.symbol_slot("second"), second_slot);

    source.accept_declaration("second Word := 4", vec!["second".into()]);
    assert!(source.declarations().contains("first Word := 3\n"));
    assert!(!source.declarations().contains("The other value"));
    assert!(!source.declarations().contains("second Word := 2"));
}

#[test]
fn replacement_removes_all_occurrences_and_type_extensions_of_the_name() {
    let mut source = SessionSource::default();
    source.accept_declaration(
        "Word is in 0 to 255\nWord has\n    finish(self): return\nvalue Word\nvalue Word := 1\nother := 2",
        vec!["Word".into(), "Word.finish".into(), "value".into(), "other".into()],
    );
    source.accept_declaration("value Word := 3", vec!["value".into()]);
    assert!(!source.declarations().contains("value Word\n"));
    assert!(!source.declarations().contains("value Word := 1"));
    assert!(
        source
            .declarations()
            .contains("Word has\n    finish(self): return\n")
    );
    let replacement = "Word is in 0 to 127\nWord has\n    finish(self): return";
    source.accept_declaration(replacement, vec!["Word".into(), "Word.finish".into()]);
    assert!(!source.declarations().contains("0 to 255"));
    assert_eq!(source.declarations().matches("Word has").count(), 1);
    assert!(source.declarations().contains("other := 2\n"));
    assert!(source.declarations().contains("value Word := 3\n"));
}

#[test]
fn partial_shared_declarations_are_preserved_and_reported() {
    let mut source = SessionSource::default();
    let original = "use 'numbers' first, second\nBox has\n    read(self): 1\n    write(self): 2";
    source.accept_declaration(
        original,
        vec![
            "first".into(),
            "second".into(),
            "Box".into(),
            "Box.read".into(),
            "Box.write".into(),
        ],
    );
    for name in ["first", "Box.read"] {
        assert!(
            source.replacement_conflict(&[name.into()]).is_some(),
            "{name}"
        );
        let candidate = source.candidate_without_shadowed("replacement := 3", &[name.into()]);
        assert!(candidate.starts_with(original));
    }
    assert!(
        source
            .replacement_conflict(&["first".into(), "second".into()])
            .is_none()
    );
    assert!(source.replacement_conflict(&["Box".into()]).is_none());
}

#[test]
fn runtime_dispatch_preserves_reference_mode_conventions() {
    let declaration = RuntimeDeclaration::new(
        "value".to_string(),
        "value(lhs ref Signed64, rhs mut Signed64) Signed64".to_string(),
        vec!["lhs".to_string(), "rhs".to_string()],
        vec![ParamConvention::Observe, ParamConvention::Mutate],
        false,
    );

    assert_eq!(
        declaration.declaration_source("__gin_live_value_generation_1", "value"),
        "__gin_live_value_generation_1(lhs ref Signed64, rhs mut Signed64) Signed64: value(lhs, rhs)\n"
    );
}

#[test]
fn runtime_dispatch_uses_bare_call_syntax_for_zero_parameter_function() {
    let declaration = RuntimeDeclaration::new(
        "value".to_string(),
        "value".to_string(),
        Vec::new(),
        Vec::new(),
        false,
    );

    assert_eq!(
        declaration.declaration_source("__gin_live_value_generation_1", "value"),
        "__gin_live_value_generation_1: value\n"
    );
}

#[test]
fn evaluation_candidate_is_transient_and_generation_named() {
    let mut source = SessionSource::default();
    source.accept_declaration("value Signed64 := 40", vec!["value".to_string()]);

    let evaluation = source.evaluation_candidate_with_dispatches("value + 2", 7, &HashMap::new());
    let expected_prefix = "__gin_interpreter_eval_7: ";
    let expression_start = evaluation
        .source()
        .find("value + 2")
        .expect("expression appears in generated source");

    assert_eq!(evaluation.symbol(), "__gin_interpreter_eval_7");
    assert_eq!(evaluation.expression(), "value + 2");
    assert_eq!(
        evaluation.expression_range(),
        expression_start..expression_start + 9,
    );
    let expected_source = format!("value Signed64 := 40\n{expected_prefix}value + 2\n");
    assert_eq!(evaluation.source(), expected_source.as_str());
    assert!(evaluation.source().contains(expected_prefix));
    assert_eq!(source.declarations(), "value Signed64 := 40\n");
}
