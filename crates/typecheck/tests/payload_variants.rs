//! `Name(Type)` in a union's `or` list declares a variant holding one value of that type.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const COUNT: &str = "#unrestricted\nCount is in 0 to 9\n\n";

#[test]
fn a_variant_with_a_payload_is_built_and_matched() {
    check(&format!(
        "{COUNT}Outcome is Opened(Count) or Failed\n\nopen(count Count) Outcome: Opened(count)\n\nsize(outcome Outcome) in 0 to 9:\n    return when outcome is\n        Opened(count) then 1\n        Failed then 0\n"
    ))
    .unwrap();
}

#[test]
fn a_generic_union_ranges_over_its_parameters() {
    check("Step(element) is Yield(element) or Done\n\nmake(element, value element) Step(element): Yield(value)\n").unwrap();
}

#[test]
fn a_linear_payload_left_in_the_other_arm_is_unconsumed() {
    // `fallback` is owned, and the `Yield` arm never uses it.
    assert_eq!(
        check("Step(element) is Yield(element) or Done\n\ntake(element, step Step(element), fallback element) element:\n    return when step is\n        Yield(value) then value\n        Done then fallback\n"),
        Err(SourceCheckError::Unconsumed)
    );
}

#[test]
fn a_payload_must_name_a_type() {
    assert!(check("Outcome is Opened(Missing) or Failed\n").is_err());
}
