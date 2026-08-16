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
fn stable_symbol_slot_is_reused_when_redeclaring() {
    let mut source = SessionSource::default();
    source.accept_declaration("value Signed64 := 1", vec!["value".to_string()]);
    let first_slot = source.symbol_slot("value").expect("slot exists");

    source.accept_declaration("value Signed64 := 2", vec!["value".to_string()]);
    let second_slot = source.symbol_slot("value").expect("slot exists");

    assert_eq!(first_slot, second_slot);
    assert_eq!(source.symbol_slots().len(), 1);
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
fn runtime_dispatch_preserves_reference_mode_conventions() {
    let declaration = RuntimeDeclaration::new(
        "value".to_string(),
        "value(ref lhs Signed64, mut rhs Signed64) Signed64".to_string(),
        vec!["lhs".to_string(), "rhs".to_string()],
        vec![ParamConvention::Observe, ParamConvention::Mutate],
        false,
    );

    assert_eq!(
        declaration.declaration_source("__gin_live_value_generation_1", "value"),
        "__gin_live_value_generation_1(ref lhs Signed64, mut rhs Signed64) Signed64: value(ref lhs, mut rhs)\n"
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
fn accepted_declarations_have_stable_trailing_newlines() {
    let mut source = SessionSource::default();

    source.accept_declaration("first := 1", vec!["first".to_string()]);

    assert_eq!(source.declarations(), "first := 1\n");
}

#[test]
fn evaluation_candidate_is_transient_and_generation_named() {
    let mut source = SessionSource::default();
    source.accept_declaration("value Signed64 := 40", vec!["value".to_string()]);

    let evaluation = source.evaluation_candidate("value + 2", 7);
    let expected_prefix = "__gin_interpreter_eval_7() Signed64: ";
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
