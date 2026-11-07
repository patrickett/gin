//! Bounded numeric dispatch and its correlation through checked interfaces.
use i256::I256;
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

const CLASSIFIER: &str = "Bit is Zero or One
classify(index in 0 to 1) Bit:
    return when index is
        0 then Zero
        1 then One
";

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid integer case syntax");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, Limits::default());
    assert_eq!(source, before);
    result
}

#[test]
fn integer_case_arms_establish_facts_about_the_actual_input() {
    let source = "echo(index in 0 to 1) in 0 to 1 and = index:
    return when index is
        0 then 0
        1 then 1
";
    check("example.echo", source, &[]).unwrap();
    let invalid = source.replace("1 then 1", "1 then 0");
    assert_ne!(invalid, source);
    assert_eq!(
        check("example.echo", &invalid, &[]).unwrap_err(),
        SourceCheckError::Unproved
    );
}

#[test]
fn repeated_matches_on_a_scalar_alias_cannot_disagree() {
    let source = "echo(index in 0 to 1) in 0 to 1 and = index:
    first: when index is
        0 then 0
        1 then 1
    saved: index
    return when saved is
        0 then first
        1 then first
";
    check("example.repeat", source, &[]).unwrap();
}

#[test]
fn numeric_outcomes_remain_correlated_through_an_imported_classifier() {
    let library = check("library.bits", CLASSIFIER, &[]).unwrap();
    let client = "use library.bits Bit, classify
identity(index in 0 to 1) in 0 to 1: return index
observe(index in 0 to 1) in 0 to 1 and = index:
    saved: index
    result: classify(identity(saved))
    return when result is
        Zero then 0
        One then 1
";
    check("example.client", client, &[&library]).unwrap();
    let wrong = client.replace("One then 1", "One then 0");
    assert_ne!(wrong, client);
    assert_eq!(
        check("example.client", &wrong, &[&library]).unwrap_err(),
        SourceCheckError::Unproved
    );
}

const RELATION: &str = "Relation is Same or Different
compare(first in 0 to 1, second in 0 to 1) Relation:
    return when first is
        0 then compare_zero(second)
        1 then compare_one(second)
compare_zero(second in 0 to 1) Relation:
    return when second is
        0 then Same
        1 then Different
compare_one(second in 0 to 1) Relation:
    return when second is
        0 then Different
        1 then Same
";

#[test]
fn aliased_actual_arguments_must_satisfy_all_callee_guards_jointly() {
    let library = check("library.relation", RELATION, &[]).unwrap();
    let client = "use library.relation Relation, compare
same(index in 0 to 1) in 0 to 1:
    result: compare(index, index)
    return when result is
        Same then 0
        Different then 1
";
    check("example.aliased", client, &[&library]).unwrap();
}

#[test]
fn missing_extra_duplicate_and_mixed_case_headers_reject() {
    for (arms, expected) in [
        ("        0 then Zero\n", SourceCheckError::Type),
        (
            "        0 then Zero\n        1 then One\n        2 then Zero\n",
            SourceCheckError::Type,
        ),
        (
            "        0 then Zero\n        0x0 then Zero\n        1 then One\n",
            SourceCheckError::Duplicate,
        ),
        (
            "        0 then Zero\n        Zero then Zero\n        1 then One\n",
            SourceCheckError::Type,
        ),
    ] {
        let source = format!(
            "Bit is Zero or One\n\
bad(index in 0 to 1) Bit:\n    return when index is\n{arms}"
        );
        let error = check("example.coverage", &source, &[]).unwrap_err();
        assert_eq!(error, expected, "{source}");
    }
}

#[test]
fn a_declared_domain_is_not_erased_when_the_actual_value_is_known() {
    let source = "zero in 0 to 1: return 0
echo in 0 to 1 and = 0:
    return when zero is
        0 then 0
        1 then 1
";
    check("example.domain", source, &[]).unwrap();
    let missing = source.replace("        1 then 1\n", "");
    assert_ne!(missing, source);
    assert_eq!(
        check("example.domain", &missing, &[]).unwrap_err(),
        SourceCheckError::Type
    );
}

#[test]
fn negative_and_signed_extreme_neighbors_do_not_overflow_coverage() {
    for lower in [I256::MIN, I256::from(-1), I256::MAX - I256::ONE] {
        let upper = lower + I256::ONE;
        let source = format!(
            "echo(index in {lower} to {upper}) in {lower} to {upper} and = index:
    return when index is
        {lower} then {lower}
        {upper} then {upper}
"
        );
        check("example.extreme", &source, &[]).unwrap();
    }
}

#[test]
fn enormous_nonexhaustive_domains_reject_without_enumerating_the_interval() {
    let source = format!(
        "Zero has
bad(index in {} to {}) Zero:
    return when index is
        {} then Zero
        {} then Zero
",
        I256::MIN,
        I256::MAX,
        I256::MIN,
        I256::MAX
    );
    let result = std::panic::catch_unwind(|| check("example.huge", &source, &[]));
    assert_eq!(result.unwrap().unwrap_err(), SourceCheckError::Type);
}

#[test]
fn integer_branches_do_not_duplicate_or_drop_captured_payloads() {
    let duplicate = "Pair(element) has left element, right element
bad(element Type, value element, other element, index in 0 to 1) Pair(element):
    return when index is
        0 then Pair(left: value, right: value)
        1 then Pair(left: value, right: other)
";
    assert_eq!(
        check("example.duplicate", duplicate, &[]).unwrap_err(),
        SourceCheckError::UseAfterMove
    );
    let dropped = "Result(element) is Empty or Full(element)
bad(element Type, value element, index in 0 to 1) Result(element):
    return when index is
        0 then Empty
        1 then Full(value)
";
    assert_eq!(
        check("example.drop", dropped, &[]).unwrap_err(),
        SourceCheckError::Unconsumed
    );
}

#[test]
fn explicit_arm_claims_use_the_selected_integer_fact() {
    let source = "echo(index in 0 to 1) out in 0 to 1 where out = index:
    return when index is
        0 then 0
        1 then 1
";
    check("example.claims", source, &[]).unwrap();
    let wrong = source.replace("where out = index", "where out = 0");
    assert_ne!(wrong, source);
    assert_eq!(
        check("example.claims", &wrong, &[]).unwrap_err(),
        SourceCheckError::Unproved
    );
}

#[test]
fn numeric_guards_survive_an_intermediate_checked_module() {
    let bits = check("library.bits", CLASSIFIER, &[]).unwrap();
    let bridge = check(
        "library.bridge",
        "use library.bits Bit, classify\nforward(index in 0 to 1) Bit: return classify(index)\n",
        &[&bits],
    )
    .unwrap();
    let client = "use library.bits Bit
use library.bridge forward
observe(index in 0 to 1) in 0 to 1 and = index:
    return when forward(index) is
        Zero then 0
        One then 1
";
    check("example.client", client, &[&bits, &bridge]).unwrap();
}

#[test]
fn guards_on_scalar_fields_remap_to_the_actual_callers_scalar_identity() {
    let bits = check("library.bits", CLASSIFIER, &[]).unwrap();
    let bridge = check(
        "library.wrapped",
        "use library.bits Bit, classify
Wrapper has index in 0 to 1
forward(wrapper Wrapper) Bit:
    Wrapper(index) := wrapper
    return classify(index)
",
        &[&bits],
    )
    .unwrap();
    let client = "use library.bits Bit
use library.wrapped Wrapper, forward
observe(index in 0 to 1) in 0 to 1 and = index:
    return when forward(Wrapper(index)) is
        Zero then 0
        One then 1
";
    check("example.client", client, &[&bits, &bridge]).unwrap();
}

#[test]
fn checked_dependency_capsules_remain_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<CheckedSourceModule>();
}

#[test]
fn declared_field_domains_survive_fresh_and_repacked_summary_outputs() {
    let source = "Wrapper has index in 0 to 1
zero Wrapper: return Wrapper(index: 0)
wrap(index in 0 to 0) Wrapper: return Wrapper(index)
identity(element Type, value element) element: return value
";
    let library = check("library.fields", source, &[]).unwrap();
    for value in ["zero", "wrap(0)", "identity(Wrapper, zero)"] {
        let client = format!(
            "use library.fields Wrapper, zero, wrap, identity
echo in 0 to 1 and = 0:
    Wrapper(index) := {value}
    return when index is
        0 then 0
        1 then 1
"
        );
        check("example.domain", &client, &[&library]).unwrap();
        let missing = client.replace("        1 then 1\n", "");
        assert_ne!(missing, client);
        assert_eq!(
            check("example.domain", &missing, &[&library]).unwrap_err(),
            SourceCheckError::Type
        );
    }
}
