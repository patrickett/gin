//! `is` tests joined with `and`, `or` and `not` in a conditional.
//!
//! A conjunction narrows with each test, and the binders of an earlier test are in scope in the
//! later ones and in the value. A disjunction or a negation could not say what a binder holds,
//! so it may combine only tests without binders.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("is.combinations", &source, &[], Limits::default())
}

fn ok(text: &str) {
    let checked = check(text);
    assert!(checked.is_ok(), "{text}\n{:?}", checked.err());
}

fn refused(text: &str) {
    assert!(check(text).is_err(), "{text}");
}

const OUTCOME: &str = "\
#unrestricted
Code is in 0 to 9

#unrestricted
Outcome is Done or Pending or Failure(Code)
";

fn function(parameters: &str, result: &str, body: &str) -> String {
    format!("{OUTCOME}\nf({parameters}) in {result}:\n    return {body}\n")
}

#[test]
fn a_conjunction_of_two_tests() {
    ok(&function(
        "a Outcome, b Outcome",
        "0 to 1",
        "when a is Done and b is Pending then 1 else 0",
    ));
}

#[test]
fn a_conjunction_binds_for_the_value() {
    ok(&function(
        "a Outcome, b Outcome",
        "0 to 9",
        "when a is Failure(x) and b is Failure(y) then x else 0",
    ));
    // The value is in scope of both, so a bound of either is usable.
    refused(&function(
        "a Outcome, b Outcome",
        "0 to 8",
        "when a is Failure(x) and b is Failure(y) then y else 0",
    ));
}

#[test]
fn a_later_test_may_use_an_earlier_binder() {
    ok(&function(
        "a Outcome, b Outcome",
        "0 to 1",
        "when a is Failure(x) and x < 5 then 1 else 0",
    ));
    ok(&function(
        "a Outcome",
        "0 to 4",
        "when a is Failure(x) and x < 5 then x else 0",
    ));
    refused(&function(
        "a Outcome",
        "0 to 3",
        "when a is Failure(x) and x < 5 then x else 0",
    ));
}

#[test]
fn a_comparison_before_a_test() {
    ok(&function(
        "a Outcome, n in 0 to 9",
        "0 to 9",
        "when n < 5 and a is Failure(x) then x else n",
    ));
}

#[test]
fn a_binder_is_not_in_scope_in_the_else() {
    refused(&function(
        "a Outcome, b Outcome",
        "0 to 9",
        "when a is Failure(x) and b is Done then 0 else x",
    ));
}

#[test]
fn a_disjunction_of_tests_without_binders() {
    ok(&function(
        "a Outcome",
        "0 to 1",
        "when a is Done or a is Pending then 1 else 0",
    ));
    ok(&function(
        "a Outcome, b Outcome",
        "0 to 1",
        "when a is Done or b is Pending then 1 else 0",
    ));
}

#[test]
fn a_test_or_a_comparison() {
    ok(&function(
        "a Outcome, n in 0 to 9",
        "0 to 1",
        "when a is Done or n < 3 then 1 else 0",
    ));
}

#[test]
fn a_disjunction_may_not_bind() {
    refused(&function(
        "a Outcome, n in 0 to 9",
        "0 to 9",
        "when a is Failure(x) or n < 3 then 1 else 0",
    ));
}

#[test]
fn a_negated_conjunction_is_a_disjunction() {
    ok(&function(
        "a Outcome, b Outcome",
        "0 to 1",
        "when not (a is Done and b is Done) then 1 else 0",
    ));
    refused(&function(
        "a Outcome, b Outcome",
        "0 to 1",
        "when not (a is Failure(x) and b is Done) then 1 else 0",
    ));
}

#[test]
fn mixing_and_with_or_is_refused() {
    refused(&function(
        "a Outcome, b Outcome, n in 0 to 9",
        "0 to 1",
        "when a is Done and b is Done or n < 3 then 1 else 0",
    ));
}

#[test]
fn both_values_are_checked() {
    refused(&function(
        "a Outcome, b Outcome",
        "0 to 1",
        "when a is Done and b is Pending then 2 else 0",
    ));
    refused(&function(
        "a Outcome, b Outcome",
        "0 to 1",
        "when a is Done and b is Pending then 1 else 2",
    ));
}

#[test]
fn a_test_in_a_chain_arm_may_be_a_conjunction() {
    ok(&function(
        "a Outcome, b Outcome, n in 0 to 9",
        "0 to 2",
        "when\n        a is Done and b is Done then 2\n        n < 3 or a is Pending then 1\n        else 0",
    ));
}
