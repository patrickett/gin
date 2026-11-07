//! A loop whose count is known only to lie in a small range is unrolled up to that range,
//! each pass behind a guard: every stopping point is a world that keeps exactly what the
//! passes before it established.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict loop fixture");
    check_file(&file, Limits::default())
}

#[test]
fn a_changed_value_is_tracked_across_every_possible_count() {
    // Every pass stores 0, whether the loop runs none, one, or six times.
    let source = |range: &str| {
        format!(
            "f(n in 0 to 5) in {range}:\n  s in 0 to 5: 0\n  for i in 0 to n do\n    s :: 0\n  return s\n"
        )
    };
    assert!(check(&source("0 to 0")).is_ok(), "{:?}", check(&source("0 to 0")));
    // The control: a body that stores 1 leaves `s` as 0 or 1, not 0.
    let ones = "f(n in 0 to 5) in 0 to 0:\n  s in 0 to 5: 0\n  for i in 0 to n do\n    s :: 1\n  return s\n";
    assert_eq!(check(ones), Err(SourceCheckError::Type));
    let ones = ones.replace("in 0 to 0", "in 0 to 1");
    assert!(check(&ones).is_ok(), "{:?}", check(&ones));
}

#[test]
fn an_accumulator_is_exact_at_each_stopping_point() {
    // At most three passes: `s` ends at 0 to 3, which the unrolled passes know and a single
    // check of the body could not.
    let source = |range: &str| {
        format!(
            "f(n in 0 to 2) in {range}:\n  s in 0 to 9: 0\n  for i in 0 to n do\n    s :: s + 1\n  return s\n"
        )
    };
    assert!(check(&source("0 to 3")).is_ok(), "{:?}", check(&source("0 to 3")));
    assert_eq!(check(&source("0 to 2")), Err(SourceCheckError::Type));
}

#[test]
fn the_counter_stays_within_what_every_pass_allows() {
    // Across all passes of `m to n` the counter is 2 to 6, and no pass goes outside it.
    let source = "\
take(x in 2 to 6): return
f(m in 2 to 4, n in 3 to 6):
  for i in m to n do
    take(i)
  return
";
    assert!(check(source).is_ok(), "{:?}", check(source));
    let tighter = source.replace("take(x in 2 to 6)", "take(x in 3 to 6)");
    assert!(check(&tighter).is_err(), "the counter can be 2");
}

#[test]
fn a_loop_that_may_not_run_is_still_a_world_without_it() {
    // With `m = 9` and `n = 0` the loop never runs, so `m` can still be 9 afterwards.
    let source = |call: &str| {
        format!(
            "take(x in 0 to 9): return\nsmall(x in 0 to 3): return\nf(m in 0 to 9, n in 0 to 3):\n  for i in m to n do\n    take(i)\n  {call}(m)\n  return\n"
        )
    };
    assert!(check(&source("take")).is_ok());
    assert_eq!(check(&source("small")), Err(SourceCheckError::Type));
}

#[test]
fn a_pass_that_moves_an_outer_owner_fails_when_a_second_pass_is_possible() {
    let source = |range: &str| {
        format!(
            "Res has v in 0 to 9\nmake() Res: return Res(v: 1)\neat(r Res):\n  Res(v) := r\n  return\nf(n in {range}):\n  r: make()\n  for i in 0 to n do\n    eat(r)\n  return\n"
        )
    };
    // Up to two passes: the second would use `r` after it was moved.
    assert_eq!(check(&source("0 to 1")), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn a_range_past_the_guarded_limit_is_still_checked_once() {
    // 0 to 999 passes cannot all be unrolled, so only the declared range survives.
    let source = "f(n in 0 to 999) in 0 to 0:\n  s in 0 to 5: 0\n  for i in 0 to n do\n    s :: 0\n  return s\n";
    assert_eq!(check(source), Err(SourceCheckError::Type));
}

#[test]
fn a_bad_body_is_reported_whichever_pass_reaches_it() {
    let source = "f(n in 0 to 3):\n  for i in 0 to n do\n    missing()\n  return\n";
    assert_eq!(check(source), Err(SourceCheckError::UnknownName));
}
