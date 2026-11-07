//! Range loops whose pass count is not one known small number: a variable bound, a count
//! beyond the unroll limit, or a branching body beyond its limit. The body is checked once
//! and must be sound for every count from zero up to the largest the types allow.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict loop fixture");
    check_file(&file, Limits::default())
}

const TAKE: &str = "\
take(x in 0 to 9): return
small(x in 0 to 3): return
";

#[test]
fn the_counter_lies_between_the_bounds_whatever_they_are_at_run_time() {
    // `m` is 2 to 4, `n` is 3 to 6: the counter can be anywhere in 2 to 6, and no further.
    let hull = "take(x in 2 to 6): return\nf(m in 2 to 4, n in 3 to 6):\n  for i in m to n do\n    take(i)\n  return\n";
    assert!(check(hull).is_ok(), "{:?}", check(hull));
    let tighter = "take(x in 3 to 6): return\nf(m in 2 to 4, n in 3 to 6):\n  for i in m to n do\n    take(i)\n  return\n";
    assert!(check(tighter).is_err(), "the counter can be 2");
    let lower = "take(x in 2 to 5): return\nf(m in 2 to 4, n in 3 to 6):\n  for i in m to n do\n    take(i)\n  return\n";
    assert!(check(lower).is_err(), "the counter can be 6");
}

#[test]
fn inside_the_body_the_bounds_are_known_to_be_ordered() {
    // The body only runs when `m <= i <= n`, so `m` is at most `n`, which is at most 3.
    let source = format!(
        "{TAKE}f(m in 0 to 9, n in 0 to 3):\n  for i in m to n do\n    small(m)\n    small(i)\n  return\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn that_ordering_does_not_survive_the_loop_which_may_not_have_run() {
    // With `m = 9` and `n = 0` the loop runs zero times, so `m` can still be 9 afterwards.
    let source = format!(
        "{TAKE}f(m in 0 to 9, n in 0 to 3):\n  for i in m to n do\n    take(i)\n  small(m)\n  return\n"
    );
    assert!(
        check(&source).is_err(),
        "m may exceed 3 after an empty loop"
    );
    let upper = format!(
        "{TAKE}f(m in 0 to 9, n in 0 to 9):\n  for i in m to n do\n    take(i)\n  small(n)\n  return\n"
    );
    assert!(check(&upper).is_err(), "n may exceed 3");
}

#[test]
fn a_fact_the_body_assumes_does_not_outlast_it() {
    // Inside, the then-world knows `x < 4`; afterwards `x` can be anything again.
    let source = format!(
        "{TAKE}f(x in 0 to 9, n in 0 to 5):\n  for i in 0 to n do\n    r: when x < 4 then 1 else 2\n  small(x)\n  return\n"
    );
    assert!(check(&source).is_err(), "x is still 0 to 9 after the loop");
}

#[test]
fn a_count_that_may_be_zero_cannot_be_relied_on_to_run() {
    // The body would move `r` if it ran, and might run once or more or never.
    let source = "\
Res has v in 0 to 9
make() Res: return Res(v: 1)
eat(r Res):
  Res(v) := r
  return
f(n in 0 to 5):
  r: make()
  for i in 0 to n do
    eat(r)
  return
";
    assert_eq!(check(source), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn an_accumulator_must_hold_on_the_largest_count() {
    // `n` can be 5, so six passes could add to 6 and leave `0 to 5`.
    let unsafe_add =
        "f(n in 0 to 5):\n  s in 0 to 5: 0\n  for i in 0 to n do\n    s :: s + 1\n  return\n";
    assert!(check(unsafe_add).is_err());
    let guarded = "f(n in 0 to 5) in 0 to 5:\n  s in 0 to 5: 0\n  for i in 0 to n do\n    s :: when s < 5 then s + 1 else 5\n  return s\n";
    assert!(check(guarded).is_ok(), "{:?}", check(guarded));
}

#[test]
fn nothing_is_known_of_a_changed_value_after_an_uncounted_loop() {
    let source = "f(n in 0 to 999) in 0 to 0:\n  s in 0 to 5: 0\n  for i in 0 to n do\n    s :: 0\n  return s\n";
    // Even though every pass stores 0, a count this large is checked once and the checker
    // only keeps the declared range.
    assert!(check(source).is_err());
    let declared = source.replace("in 0 to 0", "in 0 to 5");
    assert!(check(&declared).is_ok(), "{:?}", check(&declared));
}

#[test]
fn a_count_past_the_unroll_limit_is_sound_for_every_count() {
    let big = "f() in 0 to 5:\n  s in 0 to 5: 0\n  for i in 0 to 999 do\n    s :: when s < 5 then s + 1 else 5\n  return s\n";
    assert!(check(big).is_ok(), "{:?}", check(big));
    let exact = big.replace("in 0 to 5:\n  s", "in 5 to 5:\n  s");
    assert!(
        check(&exact).is_err(),
        "the total is not tracked past the limit"
    );
    let overflow = "f():\n  s in 0 to 5: 0\n  for i in 0 to 999 do\n    s :: s + 1\n  return\n";
    assert!(check(overflow).is_err());
}

#[test]
fn a_branching_body_past_its_limit_is_checked_once_with_both_branches() {
    // Both arms are checked even though only one runs per pass.
    let bad_arm = "f(x in 0 to 9):\n  for i in 0 to 50 do\n    r: when x < 5 then missing() else 2\n  return\n";
    assert_eq!(check(bad_arm), Err(SourceCheckError::UnknownName));
    let ok = "f(x in 0 to 9):\n  for i in 0 to 50 do\n    r: when x < 5 then 1 else 2\n  return\n";
    assert!(check(ok).is_ok(), "{:?}", check(ok));
}

#[test]
fn a_record_changed_by_the_body_is_unknown_after_an_uncounted_loop() {
    let record = "#unrestricted\nP has x in 0 to 9\npoke(p mut P):\n  p.x :: 3\n  return\n";
    let source = format!(
        "{record}f(n in 0 to 5) in 1 to 1:\n  q: P(x: 1)\n  for i in 0 to n do\n    poke(q)\n  return q.x\n"
    );
    assert!(
        check(&source).is_err(),
        "q.x is 1 when n is zero and 3 otherwise"
    );
    let known = source.replace("in 1 to 1", "in 0 to 9");
    assert!(check(&known).is_ok(), "{:?}", check(&known));
}

#[test]
fn an_immutable_value_keeps_what_was_known_across_an_uncounted_loop() {
    let source = format!(
        "{TAKE}f(n in 0 to 5) in 7 to 7:\n  k := 7\n  for i in 0 to n do\n    take(i)\n  return k\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn a_statically_empty_range_still_has_its_body_checked() {
    // `m` is at least 5 and `n` at most 3, so the loop can never run. Its body is dead code,
    // which is checked all the same (see `unreachable_code.rs`).
    let source = "f(m in 5 to 9, n in 0 to 3):\n  for i in m to n do\n    missing()\n  return\n";
    assert_eq!(check(source), Err(SourceCheckError::UnknownName));
}

#[test]
fn each_refusal_above_is_caused_by_the_one_thing_it_tests() {
    // After a possibly-empty loop `m` is only known to be 0 to 9: `take` accepts that,
    // `small` does not.
    let after_empty = |call: &str| {
        format!(
            "{TAKE}f(m in 0 to 9, n in 0 to 3):\n  for i in m to n do\n    take(i)\n  {call}(m)\n  return\n"
        )
    };
    assert!(check(&after_empty("take")).is_ok());
    assert_eq!(check(&after_empty("small")), Err(SourceCheckError::Type));

    // The assumed `x < 4` is gone: `take` fits any x, `small` does not.
    let after_fact = |call: &str| {
        format!(
            "{TAKE}f(x in 0 to 9, n in 0 to 5):\n  for i in 0 to n do\n    r: when x < 4 then 1 else 2\n  {call}(x)\n  return\n"
        )
    };
    assert!(check(&after_fact("take")).is_ok());
    assert_eq!(check(&after_fact("small")), Err(SourceCheckError::Type));

    // The stored value is only as narrow as the declared range after the loop.
    let result = |range: &str| {
        format!(
            "f(n in 0 to 999) in {range}:\n  s in 0 to 5: 0\n  for i in 0 to n do\n    s :: 0\n  return s\n"
        )
    };
    assert!(check(&result("0 to 5")).is_ok());
    assert_eq!(check(&result("0 to 0")), Err(SourceCheckError::Type));

    // Past the unroll limit the total is not tracked, only the declared range.
    let total = |range: &str| {
        format!(
            "f() in {range}:\n  s in 0 to 5: 0\n  for i in 0 to 999 do\n    s :: when s < 5 then s + 1 else 5\n  return s\n"
        )
    };
    assert!(check(&total("0 to 5")).is_ok());
    assert_eq!(check(&total("5 to 5")), Err(SourceCheckError::Type));
}
