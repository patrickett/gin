//! `when subject is Pattern then ... return`: a one-armed case.
//!
//! The subject's member is a replay choice. Where it is the pattern's variant the block runs and
//! the path ends; everywhere else the statement is passed over. A named subject is read in place,
//! so it is still there afterwards, known to be one of the other variants. A `_` binder discards
//! its field, and a field that must be consumed cannot be discarded.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.errors", &source, &[], Limits::default())
}

fn ok(text: &str) {
    let checked = check(text);
    assert!(checked.is_ok(), "{text}\n{:?}", checked.err());
}

fn refused(text: &str) -> SourceCheckError {
    check(text).expect_err(text)
}

/// A result whose failure must be used (nominal types are linear unless marked otherwise); its
/// code, a plain number, can be dropped.
const LINEAR_OUTCOME: &str = "\
#unrestricted
Code is in 0 to 9

Outcome is Done or Failure(Code)

attempt(n in 0 to 9) Outcome:
    when n < 5 then return Outcome.Done
    return Failure(n as Code)

finish(o Outcome) in 0 to 1: when o is
    Done then 0
    Failure(c) then 1
";

const FREE_OUTCOME: &str = "\
#unrestricted
Code is in 0 to 9

#unrestricted
Outcome is Done or Failure(Code)

attempt(n in 0 to 9) Outcome:
    when n < 5 then return Outcome.Done
    return Failure(n as Code)
";

#[test]
fn a_call_result_tested_for_a_failure_variant_checks() {
    ok(&format!(
        "{FREE_OUTCOME}\nrun(n in 0 to 9) in 0 to 1:\n    when attempt(n) is Failure(_) then return 1\n    return 0\n"
    ));
}

#[test]
fn the_failure_payload_may_be_bound_and_used() {
    ok(&format!(
        "{FREE_OUTCOME}\nrun(n in 0 to 9) in 0 to 9:\n    when attempt(n) is Failure(code) then\n        return code\n    return 0\n"
    ));
}

/// A named subject is read in place: where the pattern does not match it is still owned, and
/// the rest of the body consumes it.
#[test]
fn a_named_linear_subject_is_still_there_when_the_pattern_does_not_match() {
    ok(&format!(
        "{LINEAR_OUTCOME}\nrun(n in 0 to 9) in 0 to 1:\n    r: attempt(n)\n    when r is Failure(_) then return 1\n    return finish(r)\n"
    ));
}

/// Where the pattern matches, it consumes the subject: the block cannot use the name again.
#[test]
fn a_matching_pattern_consumes_a_linear_subject() {
    refused(&format!(
        "{LINEAR_OUTCOME}\nrun(n in 0 to 9) in 0 to 1:\n    r: attempt(n)\n    when r is Failure(_) then return finish(r)\n    return finish(r)\n"
    ));
}

/// An unnamed result whose other member must be consumed: testing for `Done` and passing over a
/// `Failure` would drop it, so the temporary is refused where it is passed over.
#[test]
fn an_unnamed_linear_member_cannot_be_passed_over() {
    refused(&format!(
        "{LINEAR_OUTCOME}\nrun(n in 0 to 9) in 0 to 1:\n    when attempt(n) is Done then return 1\n    return 0\n"
    ));
}

/// Where the member passed over can be dropped, the same shape checks.
#[test]
fn an_unnamed_droppable_member_can_be_passed_over() {
    ok(&format!(
        "{FREE_OUTCOME}\nrun(n in 0 to 9) in 0 to 1:\n    when attempt(n) is Done then return 1\n    return 0\n"
    ));
}

/// `_` cannot discard a field that must be consumed.
#[test]
fn a_wildcard_may_not_drop_a_linear_field() {
    let source = "\
Token has
    value in 0 to 9

#unrestricted
Outcome is Done or Failure(Token)

attempt(n in 0 to 9) Outcome:
    when n < 5 then return Outcome.Done
    return Failure(Token(value: n))

run(n in 0 to 9) in 0 to 1:
    when attempt(n) is Failure(_) then return 1
    return 0
";
    assert!(check(source).is_err());
}

#[test]
fn a_pattern_naming_a_variant_the_subject_lacks_is_refused() {
    refused(&format!(
        "{FREE_OUTCOME}\nrun(n in 0 to 9) in 0 to 1:\n    when attempt(n) is Missing(_) then return 1\n    return 0\n"
    ));
}

/// Each test is a replay choice with two outcomes, one of which ends the path, so twenty in a
/// row are twenty-one paths, not a million. (Each path replays the tests before it, so the work
/// grows with the square of the count; about thirty reach the default budget.)
#[test]
fn twenty_error_checks_in_a_row_check() {
    let mut source = format!("{FREE_OUTCOME}\nrun(n in 0 to 9) in 0 to 1:\n");
    for _ in 0..20 {
        source.push_str("    when attempt(n) is Failure(_) then return 1\n");
    }
    source.push_str("    return 0\n");
    ok(&source);
}

#[test]
fn the_returned_value_is_checked_against_the_result() {
    refused(&format!(
        "{FREE_OUTCOME}\nrun(n in 0 to 9) in 0 to 1:\n    when attempt(n) is Failure(_) then return 5\n    return 0\n"
    ));
}

#[test]
fn a_one_armed_case_that_falls_through_runs_its_block_only_for_its_member() {
    // Supported since the arm joins the worlds (`fallthrough_case.rs` has the full set).
    ok(&format!(
        "{FREE_OUTCOME}\nrun(n in 0 to 9) in 0 to 1:\n    m in 0 to 1: 0\n    when attempt(n) is Failure(_) then\n        m :: 1\n    return m\n"
    ));
    // Negative control: the fact the arm establishes is not assumed afterwards.
    assert!(matches!(
        refused(&format!(
            "{FREE_OUTCOME}\nrun(n in 0 to 9) in 1 to 1:\n    m in 0 to 1: 0\n    when attempt(n) is Failure(_) then\n        m :: 1\n    return m\n"
        )),
        SourceCheckError::Unproved | SourceCheckError::Type | SourceCheckError::Overflow
    ));
    // Still refused: an else branch on the one-armed form.
    assert!(matches!(
        refused(&format!(
            "{FREE_OUTCOME}\nrun(n in 0 to 9) in 0 to 1:\n    when attempt(n) is Failure(_) then\n        return 1\n    else\n        return 0\n"
        )),
        SourceCheckError::Unsupported | SourceCheckError::Malformed | SourceCheckError::Type
    ));
}
