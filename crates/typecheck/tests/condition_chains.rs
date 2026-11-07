//! A condition chain: `when` with ordered `condition then value` arms ending in `else value`.
//!
//! The first arm whose condition holds gives the value. Later arms are checked under the
//! assumption that the earlier conditions failed; the binders of an `is` arm are in scope only
//! in that arm's value.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("chain.cases", &source, &[], Limits::default())
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

#[test]
fn a_chain_of_comparisons_picks_the_first_arm_that_holds() {
    ok("f(n in 0 to 9) in 0 to 2:\n    return when\n        n < 3 then 0\n        n < 6 then 1\n        else 2\n");
}

#[test]
fn a_later_arm_knows_the_earlier_conditions_failed() {
    // `n >= 3` holds in the second arm and `n >= 6` in the else, so these results are exact.
    ok("f(n in 0 to 9) in 3 to 9:\n    return when\n        n < 3 then 3\n        n < 6 then n\n        else n\n");
    refused("f(n in 0 to 9) in 4 to 9:\n    return when\n        n < 3 then 4\n        n < 6 then n\n        else n\n");
    ok("f(n in 0 to 9) in 6 to 9:\n    return when\n        n < 3 then 6\n        n < 6 then 6\n        else n\n");
}

#[test]
fn every_value_is_checked_against_the_result() {
    refused("f(n in 0 to 9) in 0 to 2:\n    return when\n        n < 3 then 0\n        n < 6 then 1\n        else 3\n");
    refused("f(n in 0 to 9) in 0 to 2:\n    return when\n        n < 3 then 0\n        n < 6 then 5\n        else 2\n");
}

#[test]
fn an_arm_the_intervals_rule_out_is_not_taken() {
    // `n < 0` never holds for a natural, so the first arm's value cannot reach the result.
    ok("f(n in 0 to 9) in 0 to 2:\n    return when\n        n < 0 then 7\n        n < 5 then 1\n        else 2\n");
}

#[test]
fn an_is_arm_binds_its_fields_in_its_value() {
    ok(&format!(
        "{OUTCOME}\nf(o Outcome) in 0 to 9:\n    return when\n        o is Failure(code) then code\n        else 0\n"
    ));
}

#[test]
fn the_binder_of_one_arm_is_not_in_scope_in_another() {
    refused(&format!(
        "{OUTCOME}\nf(o Outcome) in 0 to 9:\n    return when\n        o is Done then 0\n        o is Failure(code) then 1\n        else code\n"
    ));
    refused(&format!(
        "{OUTCOME}\nf(o Outcome) in 0 to 9:\n    return when\n        o is Failure(code) then 1\n        o is Done then code\n        else 0\n"
    ));
}

#[test]
fn the_then_may_follow_on_the_next_line() {
    ok(&format!(
        "{OUTCOME}\nf(o Outcome, n in 0 to 9) in 0 to 9:\n    return when\n        o is Failure(code)\n        then code\n        n < 3\n        then 1\n        else 0\n"
    ));
}

#[test]
fn is_arms_mix_with_comparison_arms() {
    ok(&format!(
        "{OUTCOME}\nf(o Outcome, n in 0 to 9) in 0 to 9:\n    return when\n        n < 3 then 0\n        o is Failure(code) then code\n        else n\n"
    ));
}

#[test]
fn a_negative_test_without_binders_is_an_arm() {
    ok(&format!(
        "{OUTCOME}\nf(o Outcome) in 0 to 1:\n    return when\n        not o is Done then 1\n        else 0\n"
    ));
}

#[test]
fn a_chain_in_a_local() {
    ok("f(n in 0 to 9) in 0 to 2:\n    x in 0 to 2: when\n        n < 3 then 0\n        n < 6 then 1\n        else 2\n    return x\n");
    refused("f(n in 0 to 9) in 0 to 2:\n    x in 0 to 1: when\n        n < 3 then 0\n        n < 6 then 1\n        else 2\n    return x\n");
}

#[test]
fn a_call_subject_is_tested_in_order() {
    ok(&format!(
        "{OUTCOME}\nattempt(n in 0 to 9) Outcome:\n    when n < 5 then return Outcome.Done\n    return Failure(n as Code)\n\nf(n in 0 to 9) in 0 to 9:\n    return when\n        n < 2 then 0\n        attempt(n) is Failure(code)\n        then code\n        else 1\n"
    ));
}

/// The shape of `from_num` in the terminal's `io.gin`: comparison arms, then an `else`, each
/// building a variant.
#[test]
fn a_chain_builds_a_variant_from_the_number() {
    ok(&format!(
        "{OUTCOME}\nmake(n in 0 to 9) Outcome:\n    return when\n        n = 0 then Outcome.Done\n        n < 5 then Failure(n as Code)\n        else Outcome.Pending\n"
    ));
}

/// All the values must agree on one type: here one arm is a number.
#[test]
fn arms_of_different_types_are_refused() {
    refused(&format!(
        "{OUTCOME}\nmake(n in 0 to 9) Outcome:\n    return when\n        n = 0 then Outcome.Done\n        n < 5 then 7\n        else Outcome.Pending\n"
    ));
}

/// A chain used where a number is expected must give a number in every arm.
#[test]
fn a_chain_arm_naming_a_missing_name_is_refused() {
    refused("f(n in 0 to 9) in 0 to 2:\n    return when\n        n < 3 then missing\n        else 2\n");
}
