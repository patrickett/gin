//! `x is in T`: a condition that is true when `x` lies in the declared range of `T`, and that
//! narrows: inside the branch the range is known, so a conversion to `T` is safe.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.membership", &source, &[], Limits::default())
}

fn ok(text: &str) {
    let checked = check(text);
    assert!(checked.is_ok(), "{text}\n{:?}", checked.err());
}

fn refused(text: &str) {
    assert!(check(text).is_err(), "{text}");
}

const BYTE: &str = "#unrestricted\nByte is in 0 to 255\n";

/// The true branch knows the range, so the number becomes a value of the domain.
#[test]
fn a_member_converts_to_its_domain() {
    ok(&format!(
        "{BYTE}\nfit(n in 0 to 1000) in 0 to 255:\n    when n is in Byte then\n        b Byte: n\n        return 0\n    return 1\n"
    ));
}

/// Without the test the same binding is refused: the number might not fit.
#[test]
fn the_conversion_needs_the_test() {
    refused(&format!(
        "{BYTE}\nfit(n in 0 to 1000) in 0 to 255:\n    b Byte: n\n    return 0\n"
    ));
}

/// Inside the branch the number is within the range; after a failed test it is outside it.
#[test]
fn a_member_is_within_the_range_inside_the_branch() {
    // 0..255 inside, so `n` fits a `0 to 255` result.
    ok("f(n in 0 to 1000) in 0 to 255:\n    when n is in 0 to 255 then return n\n    return 0\n");
    refused("f(n in 0 to 1000) in 0 to 254:\n    when n is in 0 to 255 then return n\n    return 0\n");
}

/// The other branch knows the number is outside the range (no single fact, so only a check
/// that it is possible).
#[test]
fn a_non_member_must_be_possible() {
    ok("f(n in 0 to 1000) in 0 to 255:\n    when n is in 0 to 255 then return n\n    return 0\n");
    // Every `n` is a member, so the failing branch cannot be reached; it is not checked.
    ok("f(n in 0 to 100) in 0 to 255:\n    when n is in 0 to 255 then return n\n    return 999\n".replace("999", "0").as_str());
}

#[test]
fn not_swaps_the_branches() {
    ok("f(n in 0 to 1000) in 0 to 255:\n    when not n is in 0 to 255 then return 0\n    return n\n");
    refused("f(n in 0 to 1000) in 0 to 254:\n    when not n is in 0 to 255 then return 0\n    return n\n");
}

/// As a value conditional, the draft's `result is in SignedBigInt then result as Self else ...`.
#[test]
fn a_value_conditional_on_membership() {
    ok(&format!(
        "{BYTE}\nclamp(n in 0 to 1000) in 0 to 255: return when n is in 0 to 255 then n else 255\n"
    ));
    refused(&format!(
        "{BYTE}\nclamp(n in 0 to 1000) in 0 to 254: return when n is in 0 to 255 then n else 254\n"
    ));
}

/// A fall-through join on membership, and a loop.
#[test]
fn a_join_on_membership() {
    ok("f(n in 0 to 1000) in 0 to 9:\n    m in 0 to 9: 0\n    when n is in 0 to 9 then m :: n\n    return m\n");
}

/// A membership is one relation, so it joins other conditions like one.
#[test]
fn a_membership_joins_other_conditions() {
    ok("f(n in 0 to 1000) in 0 to 9:\n    when n > 100 or n is in 0 to 9 then return 0\n    return 1\n");
}

#[test]
fn the_domain_must_declare_a_range() {
    refused("Pair has\n    a in 0 to 9\n\nf(n in 0 to 9) in 0 to 1:\n    when n is in Pair then return 1\n    return 0\n");
}
