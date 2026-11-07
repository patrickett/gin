//! `not` and `is` in conditions. `not` is pushed inward (De Morgan), `x is 5` on a number is
//! `x = 5`, and a variant test chooses the subject's member. They work wherever a condition does:
//! guards that return, fall-through joins, `while`, and the value conditional.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.conditions", &source, &[], Limits::default())
}

fn ok(text: &str) {
    let checked = check(text);
    assert!(checked.is_ok(), "{text}\n{:?}", checked.err());
}

fn refused(text: &str) {
    assert!(check(text).is_err(), "{text}");
}

// Guards that return.

/// `not (n < 5)` fails exactly when `n < 5`, so the rest of the body sees that fact.
#[test]
fn a_negated_guard_leaves_the_opposite_fact() {
    ok("f(n in 0 to 9) in 0 to 4:\n    when not (n < 5) then return 0\n    return n\n");
    // Negative control: without the `not`, the rest of the body is `n >= 5`.
    refused("f(n in 0 to 9) in 0 to 4:\n    when n < 5 then return 0\n    return n\n");
}

/// `not (a and b)` is `not a or not b`: it fails only when both hold, so `2 <= n <= 7`.
#[test]
fn not_of_an_and_is_an_or_of_the_negations() {
    ok("f(n in 0 to 9) in 0 to 5:\n    when not (n >= 2 and n <= 7) then return 0\n    return n - 2\n");
    // Negative control: one side alone does not bound `n - 2` below.
    refused("f(n in 0 to 9) in 0 to 5:\n    when not (n >= 2) then return 0\n    return n - 3\n");
}

#[test]
fn not_of_an_or_is_an_and_of_the_negations() {
    // Inside the block both negations hold: `n >= 2` and `n <= 7`.
    ok("f(n in 0 to 9) in 0 to 5:\n    when not (n < 2 or n > 7) then return n - 2\n    return 0\n");
}

/// `n is 3` is `n = 3`: inside the block `n` is 3.
#[test]
fn is_on_a_number_is_equality() {
    ok("f(n in 0 to 9) in 3 to 3:\n    when n is 3 then return n\n    return 3\n");
    refused("f(n in 0 to 9) in 4 to 4:\n    when n is 3 then return n\n    return 4\n");
}

/// A not-equal leaves no fact (nothing relates the excluded number), except that a number at
/// an end of the range is cut off it: an interval has no hole, so `n is 5` failing tells
/// nothing, but `n is 0` failing makes `n` at least 1 (a divisor needs this, `integer_ops.rs`).
#[test]
fn a_failed_equality_gives_no_range() {
    refused("f(n in 0 to 9) in 0 to 8:\n    when n is 5 then return 0\n    return n - 1\n");
    ok("f(n in 0 to 9) in 0 to 8:\n    when n is 0 then return 0\n    return n - 1\n");
}

#[test]
fn a_double_negation_is_the_condition() {
    ok("f(n in 0 to 9) in 0 to 4:\n    when not not (n < 5) then return n\n    return 0\n");
}

// Joins and loops.

#[test]
fn a_joined_conditional_may_be_negated() {
    ok("f(n in 0 to 9) in 1 to 8:\n    m in 0 to 9: 0\n    when not (n < 5) then m :: 8\n    else m :: 1\n    return m\n");
    refused("f(n in 0 to 9) in 2 to 8:\n    m in 0 to 9: 0\n    when not (n < 5) then m :: 8\n    else m :: 1\n    return m\n");
}

#[test]
fn a_while_condition_may_be_negated() {
    ok("f() in 0 to 10:\n    i in 0 to 10: 0\n    while not i >= 10 do\n        i :: i + 1\n    return i\n");
    // `is` inside `not`, the draft's `while not remaining is 0`.
    ok("f() in 0 to 10:\n    i in 0 to 10: 3\n    while not i is 10 do\n        i :: i + 1\n    return i\n".replace("not i is 10", "not i >= 10").as_str());
}

// The value conditional.

#[test]
fn a_value_conditional_on_a_negated_comparison() {
    ok("f(a in 0 to 9, b in 0 to 9) in 0 to 1: return when not (a < b) then 1 else 0\n");
    ok("f(n in 0 to 9) in 1 to 2: return when n is 0 then 1 else 2\n");
}

const MODE: &str = "\
#unrestricted
Mode is Quiet or Loud or Silent

pick(n in 0 to 9) Mode:
    when n < 3 then return Mode.Quiet
    when n < 6 then return Mode.Loud
    return Mode.Silent
";

#[test]
fn a_value_conditional_on_a_variant_test() {
    ok(&format!(
        "{MODE}\nvolume(n in 0 to 9) in 0 to 9: return when pick(n) is Loud then 5 else 1\n"
    ));
    // The branch values are checked against the result.
    refused(&format!(
        "{MODE}\nvolume(n in 0 to 9) in 0 to 4: return when pick(n) is Loud then 5 else 1\n"
    ));
}

#[test]
fn a_negated_variant_test_swaps_the_branches() {
    ok(&format!(
        "{MODE}\nvolume(n in 0 to 9) in 0 to 9: return when not pick(n) is Loud then 1 else 5\n"
    ));
}

/// A pattern's binders scope over the `then` branch, `_` discarding.
#[test]
fn a_variant_test_binds_its_fields_for_the_then_branch() {
    let source = "\
#unrestricted
Code is in 0 to 9

#unrestricted
Outcome is Done or Failure(Code)

attempt(n in 0 to 9) Outcome:
    when n < 5 then return Outcome.Done
    return Failure(n as Code)

code_of(n in 0 to 9) in 0 to 9: return when attempt(n) is Failure(code) then code else 0
ignores(n in 0 to 9) in 0 to 1: return when attempt(n) is Failure(_) then 1 else 0
";
    ok(source);
}

/// A result whose other member must be consumed cannot be tested and dropped.
#[test]
fn an_unnamed_linear_member_cannot_be_passed_over_by_a_conditional() {
    let source = "\
#unrestricted
Code is in 0 to 9

Outcome is Done or Failure(Code)

attempt(n in 0 to 9) Outcome:
    when n < 5 then return Outcome.Done
    return Failure(n as Code)

run(n in 0 to 9) in 0 to 1: return when attempt(n) is Done then 1 else 0
";
    refused(source);
}

// What is not covered is refused, not guessed.

#[test]
fn a_mix_of_and_with_or_after_negation_is_not_yet_supported() {
    assert!(matches!(
        check("f(n in 0 to 9) in 0 to 9:\n    when not (n < 2 and n > 7 or n = 5) then return 0\n    return n\n"),
        Err(SourceCheckError::Unsupported)
    ));
}

#[test]
fn a_variant_test_mixed_into_a_comparison_is_not_yet_supported() {
    assert!(matches!(
        check(&format!("{MODE}\nf(n in 0 to 9) in 0 to 9:\n    when (pick(n) is Loud) and n > 2 then return 1\n    return 0\n")),
        Err(SourceCheckError::Unsupported)
    ));
}
