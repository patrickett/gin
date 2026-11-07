//! A guard on a case arm: `Pattern and condition then value`.
//!
//! The pattern's binders are in scope in the guard and the body. An arm with a guard does not
//! cover its member, so another arm without one must; a guard that fails falls to the next arm
//! for the member.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.guards", &source, &[], Limits::default())
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

fn case(arms: &str, result: &str) -> String {
    format!("{OUTCOME}\nf(o Outcome) in {result}: when o is\n{arms}")
}

#[test]
fn a_guarded_arm_falls_to_an_arm_without_one() {
    ok(&case(
        "    Done then 0\n    Failure(c) and c < 5 then 1\n    Failure(c) then 2\n",
        "0 to 2",
    ));
}

#[test]
fn the_binders_are_in_scope_in_the_guard_and_the_body() {
    // `c < 5` is assumed in the body, so `c` fits `0 to 4`.
    ok(&case(
        "    Done then 0\n    Failure(c) and c < 5 then c\n    Failure(c) then 0\n",
        "0 to 4",
    ));
    refused(&case(
        "    Done then 0\n    Failure(c) and c < 6 then c\n    Failure(c) then 0\n",
        "0 to 4",
    ));
}

#[test]
fn the_arm_after_the_guard_is_still_checked() {
    refused(&case(
        "    Done then 0\n    Failure(c) and c < 5 then c\n    Failure(c) then c\n",
        "0 to 4",
    ));
}

#[test]
fn several_guards_try_in_order() {
    ok(&case(
        "    Done then 0\n    Failure(c) and c < 3 then 1\n    Failure(c) and c < 6 then 2\n    Failure(c) then 3\n",
        "0 to 3",
    ));
    refused(&case(
        "    Done then 0\n    Failure(c) and c < 3 then 1\n    Failure(c) and c < 6 then 4\n    Failure(c) then 3\n",
        "0 to 3",
    ));
}

#[test]
fn a_guard_does_not_cover_its_member() {
    refused(&case("    Done then 0\n    Failure(c) and c < 5 then 1\n", "0 to 1"));
    // The unguarded arm may come first only when nothing follows it for the member.
    refused(&case(
        "    Done then 0\n    Failure(c) then 2\n    Failure(c) and c < 5 then 1\n",
        "0 to 2",
    ));
}

#[test]
fn a_guard_on_the_only_arm_for_a_member_still_needs_a_default() {
    refused(&case("    Done and 1 < 2 then 0\n    Failure(c) then 1\n", "0 to 1"));
}

#[test]
fn a_binder_of_another_arm_is_not_in_scope_in_a_guard() {
    refused(&case(
        "    Done and c < 5 then 0\n    Done then 0\n    Failure(c) then 1\n",
        "0 to 1",
    ));
}

#[test]
fn two_arms_without_guards_for_one_member_are_refused() {
    refused(&case(
        "    Done then 0\n    Failure(c) then 1\n    Failure(c) then 2\n",
        "0 to 2",
    ));
}

#[test]
fn a_guard_may_compare_the_binder_with_a_parameter() {
    ok(&format!(
        "{OUTCOME}\nf(o Outcome, limit in 0 to 9) in 0 to 2: when o is\n    Done then 0\n    Failure(c) and c < limit then 1\n    Failure(c) then 2\n"
    ));
}

#[test]
fn a_guarded_case_in_a_statement() {
    ok(&format!(
        "{OUTCOME}\nf(o Outcome) in 0 to 3:\n    when o is\n        Done then 3\n        Failure(c) and c < 5 then 1\n        Failure(c) then 2\n"
    ));
}

#[test]
fn a_guard_that_names_a_missing_name_is_refused() {
    refused(&case(
        "    Done then 0\n    Failure(c) and missing < 5 then 1\n    Failure(c) then 2\n",
        "0 to 2",
    ));
}

#[test]
fn a_guard_may_be_an_is_test_of_the_binder() {
    ok(&case(
        "    Done then 0\n    Failure(c) and c is 3 then 1\n    Failure(c) then 2\n",
        "0 to 2",
    ));
}
