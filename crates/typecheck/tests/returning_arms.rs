//! A case arm whose body is `return`: the path ends, and the arm has no value.
//!
//! The arm leaves the function with its value, typed against the function's declared result.
//! The other arms give the case's value, or go on to the statements after it.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.arms", &source, &[], Limits::default())
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
Outcome is Done or Failure(Code)
";

#[test]
fn a_returning_arm_beside_an_arm_with_a_value() {
    ok(&format!(
        "{OUTCOME}\nfinish(o Outcome) in 0 to 9: when o is\n    Done then 1\n    Failure(c) then return 5\n"
    ));
}

/// The returned value is checked against the function's declared result, not the case's.
#[test]
fn the_returned_value_is_checked_against_the_result() {
    refused(&format!(
        "{OUTCOME}\nfinish(o Outcome) in 0 to 4: when o is\n    Done then 1\n    Failure(c) then return 5\n"
    ));
}

/// The case is not the last statement: the returning arm leaves the function, and the others
/// go on to what follows.
#[test]
fn a_returning_arm_in_a_statement_leaves_the_function() {
    let body = "    when o is\n        Done then 7\n        Failure(c) then return 5\n    return 2\n";
    ok(&format!("{OUTCOME}\nrun(o Outcome) in 2 to 5:\n{body}"));
    // Both exits are in the result: 5 from the arm, 2 from after the case.
    refused(&format!("{OUTCOME}\nrun(o Outcome) in 2 to 4:\n{body}"));
    refused(&format!("{OUTCOME}\nrun(o Outcome) in 3 to 5:\n{body}"));
}

/// An arm that returns binds its fields like any other, and may use them.
#[test]
fn a_returning_arm_may_use_the_pattern_binders() {
    ok(&format!(
        "{OUTCOME}\nrun(o Outcome) in 0 to 9:\n    when o is\n        Done then 0\n        Failure(code) then return code\n    return 0\n"
    ));
}

/// Every arm may return: the case then has no value, and nothing follows it.
#[test]
fn every_arm_may_return() {
    ok(&format!(
        "{OUTCOME}\nrun(o Outcome) in 0 to 9: when o is\n    Done then return 1\n    Failure(c) then return 2\n"
    ));
}

/// A second case on the same value has its member already fixed by the first, so its other
/// arms are checked in a detached world no real path takes. A `return` there ends only that
/// check, and its value is still held to the result.
#[test]
fn a_returning_arm_in_a_detached_check_is_still_checked() {
    let twice = |value: i32| {
        format!(
            "{OUTCOME}\nrun(o Outcome) in 0 to 4:\n    when o is\n        Done then 1\n        Failure(c) then 2\n    when o is\n        Done then 1\n        Failure(c) then return {value}\n    return 0\n"
        )
    };
    ok(&twice(4));
    refused(&twice(9));
}

/// A unit-valued method whose arms release: the draft's `Pushed then return`.
#[test]
fn a_bare_return_arm_in_a_unit_method() {
    ok("\
#unrestricted
Code is in 0 to 9
#unrestricted
PushResult is Pushed or Failed(Code)

PushResult has
    release(self):
        when self is
            Pushed then return
            Failed(code) then return
");
}
