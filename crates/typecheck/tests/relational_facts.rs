//! Relations between numbers (`linear.rs`): `n <= count` shows `count - n >= 0`, a record's claim
//! `first + count <= max` survives destructuring and rebuilding, and facts are about immutable
//! numbers so a write or a `mut` transition leaves them true only of the old value.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict fixture");
    check_file(&file, Limits::default()).map(|_| ())
}

/// A refusal: a value that may leave its declared range is `Type` for a plain range and
/// `Unproved` where a claim is checked; either way the program is not accepted.
fn refused(text: &str) {
    assert!(
        matches!(check(text), Err(SourceCheckError::Type | SourceCheckError::Unproved)),
        "{text}\n{:?}",
        check(text)
    );
}

fn accepted(text: &str) {
    assert_eq!(check(text), Ok(()), "{text}");
}

fn with(tail: &str) -> String {
    format!("{tail}\nmain(): return\n")
}

#[test]
fn a_difference_is_not_negative_when_the_bound_is_known() {
    accepted(&with("shrink(count in 0 to 1000, n in 0 to 1000 and n <= count) in 0 to 1000: return count - n"));
    accepted(&with("shrink(count in 0 to 1000, n in 0 to 1000 and n < count) in 1 to 1000: return count - n"));
    // A guard teaches the same fact.
    accepted(&with(
        "shrink(count in 0 to 1000, n in 0 to 1000) in 0 to 1000:\n    when n <= count then return count - n\n    return 0",
    ));
}

#[test]
fn a_difference_is_refused_in_the_wrong_direction_or_without_enough_bound() {
    // Wrong direction: `count <= n` says nothing of `count - n >= 0`.
    refused(&with("shrink(count in 0 to 1000, n in 0 to 1000 and count <= n) in 0 to 1000: return count - n"));
    // Insufficient: `n <= count` allows `count - n = 0`, so a result of at least 1 is not shown.
    refused(&with("shrink(count in 0 to 1000, n in 0 to 1000 and n <= count) in 1 to 1000: return count - n"));
    // No fact at all.
    refused(&with("shrink(count in 0 to 1000, n in 0 to 1000) in 0 to 1000: return count - n"));
}

const PAIR: &str = "\
#unrestricted
Pair has
    first in 0 to 1000
    count in 0 to 1000 and first + count <= 1000

";

fn pair(methods: &str) -> String {
    format!("{PAIR}{methods}\nmain(): return\n")
}

#[test]
fn a_record_claim_survives_destructuring_and_rebuilding() {
    accepted(&pair(
        "    same(self) Pair:\n        Pair(first, count) := self\n        return Pair(first: first, count: count)\n\n    one(self and self.count = 1) Pair:\n        Pair(first, count) := self\n        return Pair(first: first, count: 1)\n\n    front(self, n in 0 to 1000 and n <= self.count) Pair:\n        Pair(first, count) := self\n        return Pair(first: first, count: n)\n",
    ));
}

#[test]
fn a_rebuilt_record_must_still_show_its_claim() {
    // One more than it had.
    refused(&pair("    more(self) Pair:\n        Pair(first, count) := self\n        return Pair(first: first, count: count + 1)\n"));
    // The same count with a first moved up.
    refused(&pair("    later(self) Pair:\n        Pair(first, count) := self\n        return Pair(first: first + 1, count: count)\n"));
}

#[test]
fn splitting_moves_the_boundary_with_checked_arithmetic() {
    let split = "    split(self, n in 0 to 1000 and n <= self.count) front Pair, rest Pair:\n        Pair(first, count) := self\n        return Pair(first: first, count: n), Pair(first: first + n, count: count - n)\n";
    accepted(&pair(split));
    // Taking more than there is leaves no room: the second half is not shown to fit.
    refused(&pair(&split.replace("n <= self.count", "n <= 1000")));
}

#[test]
fn a_fact_is_about_the_old_number_after_a_write() {
    // `a <= b`, then `a` is rewritten: `b - a` is no longer shown non-negative.
    refused(&with("f(a mut in 0 to 1000, b in 0 to 1000 and a <= b) in 0 to 1000:\n    a :: 1000\n    return b - a"));
    // Control: reading before the write is fine.
    accepted(&with(
        "f(a mut in 0 to 1000, b in 0 to 1000 and a <= b) in 0 to 1000:\n    gap in 0 to 1000: b - a\n    a :: 1000\n    return gap",
    ));
}

#[test]
fn a_transition_replaces_the_fact_and_an_unrelated_place_keeps_it() {
    // `bump` names `a` in its transition; `b` and `c` are not named, so their facts stay.
    let calls = "bump(a mut in 0 to 2000) where a :: 7:\n    a :: 7\n    return\n\n";
    // Between `a` and `b`: stale after the call.
    refused(&with(&format!(
        "{calls}g(a mut in 0 to 2000, b in 0 to 1000 and a <= b) in 0 to 1000:\n    bump(a)\n    return b - a"
    )));
    // Between `b` and `c`, neither of which the call touches: still true.
    accepted(&with(&format!(
        "{calls}h(a mut in 0 to 2000, b in 0 to 1000, c in 0 to 1000 and b <= c) in 0 to 1000:\n    bump(a)\n    return c - b"
    )));
}

#[test]
fn a_call_that_writes_without_saying_so_forgets_the_place() {
    refused(&with(
        "clobber(a mut in 0 to 1000):\n    a :: 0\n    return\n\nk(a mut in 0 to 1000, b in 0 to 1000 and a <= b) in 0 to 1000:\n    clobber(a)\n    return b - a",
    ));
}

#[test]
fn the_same_product_written_again_is_the_same_number() {
    // `a * b` in the callee's claim is the `a * b` the caller computed.
    let need = "need(a in 0 to 9, b in 0 to 9, p in 0 to 81 and p = a * b): return\n";
    accepted(&with(&format!("{need}ok(a in 0 to 9, b in 0 to 9):\n    p in 0 to 81: a * b\n    need(a, b, p)\n    return")));
    // Either order of the factors.
    accepted(&with(&format!("{need}ok(a in 0 to 9, b in 0 to 9):\n    p in 0 to 81: b * a\n    need(a, b, p)\n    return")));
    // Negative controls: another product, or a different number, is not that product.
    refused(&with(&format!("{need}no(a in 0 to 9, b in 0 to 9, c in 0 to 9):\n    p in 0 to 81: a * c\n    need(a, b, p)\n    return")));
    refused(&with(&format!("{need}no(a in 0 to 9, b in 0 to 9, p in 0 to 81):\n    need(a, b, p)\n    return")));
}
