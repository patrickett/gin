//! `for i in lo to hi do body`: the body is checked once, against the weakest state any
//! earlier iteration could have left, so it holds for every number of iterations.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict loop fixture");
    check_file(&file, Limits::default())
}

const TAKE: &str = "take(x in 0 to 9): return\n";

#[test]
fn the_counter_ranges_over_the_bounds() {
    let source = format!("{TAKE}f():\n  for i in 0 to 9 do\n    take(i)\n  return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
    let too_wide = "take(x in 0 to 8): return\nf():\n  for i in 0 to 9 do\n    take(i)\n  return\n";
    assert!(check(too_wide).is_err(), "9 does not fit 0 to 8");
}

#[test]
fn a_bound_that_is_not_a_constant_limits_the_counter_by_its_range() {
    let source =
        "take(x in 0 to 5): return\nf(n in 0 to 5):\n  for i in 0 to n do\n    take(i)\n  return\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
    let wider =
        "take(x in 0 to 4): return\nf(n in 0 to 5):\n  for i in 0 to n do\n    take(i)\n  return\n";
    assert!(check(wider).is_err());
}

#[test]
fn the_counter_cannot_be_reassigned() {
    let source = "f():\n  for i in 0 to 3 do\n    i :: 1\n  return\n";
    assert_eq!(check(source), Err(SourceCheckError::Immutable));
}

#[test]
fn a_range_that_is_certainly_empty_never_runs_its_body() {
    let source = format!("{TAKE}f():\n  for i in 5 to 1 do\n    take(i)\n  return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn loops_nest() {
    let source = format!(
        "{TAKE}f():\n  for i in 0 to 2 do\n    for j in 0 to 2 do\n      take(i)\n      take(j)\n  return\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

const RES: &str = "\
Res has v in 0 to 9
look(r ref Res): return
poke(r mut Res): return
eat(r Res):
  Res(v) := r
  return
make() Res: return Res(v: 1)
";

#[test]
fn an_outer_owner_can_be_lent_on_every_pass_and_ended_after() {
    let source = format!(
        "{RES}f():\n  r: make()\n  for i in 0 to 3 do\n    look(r)\n    poke(r)\n  eat(r)\n  return\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn an_outer_owner_cannot_be_ended_inside_the_loop() {
    let source = format!("{RES}f():\n  r: make()\n  for i in 0 to 3 do\n    eat(r)\n  return\n");
    assert_eq!(check(&source), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn an_outer_owner_left_unended_is_still_an_error_after_the_loop() {
    let source = format!("{RES}f():\n  r: make()\n  for i in 0 to 3 do\n    look(r)\n  return\n");
    assert_eq!(check(&source), Err(SourceCheckError::Unconsumed));
}

#[test]
fn what_one_pass_makes_it_must_end() {
    let leaked = format!("{RES}f():\n  for i in 0 to 3 do\n    r: make()\n    look(r)\n  return\n");
    assert_eq!(check(&leaked), Err(SourceCheckError::Unconsumed));
    let ended = format!("{RES}f():\n  for i in 0 to 3 do\n    r: make()\n    eat(r)\n  return\n");
    assert!(check(&ended).is_ok(), "{:?}", check(&ended));
}

/// A `return` inside a range loop's body ends the path with its value.
#[test]
fn a_returned_value_inside_the_body_ends_the_path() {
    let source = "f() in 0 to 1:\n  for i in 0 to 3 do\n    return 1\n  return 0\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
    // It is typed against the function's result, not the loop's.
    let outside = "f() in 0 to 0:\n  for i in 0 to 3 do\n    return 1\n  return 0\n";
    assert!(check(outside).is_err());
}

#[test]
fn a_return_in_an_uncounted_range_does_not_hide_the_empty_path() {
    let source = "f(n in 0 to 100) in 0 to 0:\n  for i in 1 to n do\n    return 0\n  return 1\n";
    assert_eq!(check(source).unwrap_err(), SourceCheckError::Type);
    let valid = source.replace("return 1\n", "return 0\n");
    assert!(check(&valid).is_ok(), "{:?}", check(&valid));
    let entered = source.replace("n in 0 to 100", "n in 1 to 100");
    assert!(check(&entered).is_ok(), "{:?}", check(&entered));
    let skipped = source.replace("n in 0 to 100", "n in 0 to 0");
    assert_eq!(check(&skipped).unwrap_err(), SourceCheckError::Type);
    let invariant = source.replace("1 to n do", "1 to n where i >= 1 do");
    assert_eq!(check(&invariant).unwrap_err(), SourceCheckError::Type);
    let valid_invariant = invariant.replace("return 1\n", "return 0\n");
    assert!(
        check(&valid_invariant).is_ok(),
        "{:?}",
        check(&valid_invariant)
    );
    // The bounded, guarded strategy must keep the same skipped-path behavior.
    let guarded = source.replace("n in 0 to 100", "n in 0 to 3");
    assert_eq!(check(&guarded).unwrap_err(), SourceCheckError::Type);
    let valid_guarded = guarded.replace("return 1\n", "return 0\n");
    assert!(check(&valid_guarded).is_ok(), "{:?}", check(&valid_guarded));
}

#[test]
fn a_counter_can_be_updated_each_pass_within_its_declared_range() {
    let source = "\
f() in 0 to 100:
  n in 0 to 100: 0
  for i in 0 to 9 do
    n :: when n < 100 then n + 1 else 100
  return n
";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn what_was_known_before_a_loop_that_changes_it_is_forgotten() {
    let source = "\
f() in 0 to 0:
  n in 0 to 100: 0
  for i in 0 to 9 do
    n :: when n < 100 then n + 1 else 100
  return n
";
    assert!(check(source).is_err(), "n may no longer be 0");
}

#[test]
fn an_update_that_leaves_the_range_on_some_pass_is_refused() {
    // Exactly counted: from 95 the tenth pass would reach 105.
    let counted = "f():\n  n in 0 to 100: 95\n  for i in 0 to 9 do\n    n :: n + 1\n  return\n";
    assert!(check(counted).is_err());
    // Too many passes to count, so any pass could start from 100.
    let uncounted = "f():\n  n in 0 to 100: 0\n  for i in 0 to 1000 do\n    n :: n + 1\n  return\n";
    assert!(check(uncounted).is_err());
}

#[test]
fn what_cannot_change_in_the_loop_keeps_what_was_known() {
    let kept =
        format!("{TAKE}f() in 5 to 5:\n  k := 5\n  for i in 0 to 3 do\n    take(i)\n  return k\n");
    assert!(check(&kept).is_ok(), "{:?}", check(&kept));
}

#[test]
fn a_record_lent_mut_in_the_loop_is_forgotten_after_it() {
    let record = "#unrestricted\nP has x in 0 to 9\npoke(p mut P):\n  p.x :: 3\n  return\n";
    let known = format!(
        "{record}f() in 0 to 9:\n  q: P(x: 1)\n  for i in 0 to 2 do\n    poke(q)\n  return q.x\n"
    );
    assert!(check(&known).is_ok(), "{:?}", check(&known));
    let assumed = known.replace("f() in 0 to 9", "f() in 1 to 1");
    assert!(check(&assumed).is_err(), "q.x may have changed");
}

#[test]
fn a_shared_borrow_is_not_changed_by_the_loop() {
    let record = "#unrestricted\nP has x in 0 to 9\nlook(p ref P): return\n";
    let source =
        format!("{record}f(q ref P) in 0 to 9:\n  for i in 0 to 2 do\n    look(q)\n  return q.x\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn a_loop_over_a_fixed_sequence_still_consumes_it() {
    let source = "\
Token has number in 0 to 16
Element is Empty or Token(Token)
finish(token Token):
    Token(number) := token
    return
consume(value Element):
    return when value is
        Empty then ()
        Token(token) then finish(token)
drain(values (Element, Element)):
    for value in values do
        consume(value)
    return
";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn a_counted_loop_is_recorded_once_unrolled() {
    let source = format!("{TAKE}f():\n  for i in 0 to 3 do\n    take(i)\n  return\n");
    let checked = check(&source).unwrap();
    let index = checked
        .functions
        .iter()
        .position(|f| f.name == "f")
        .unwrap();
    assert!(checked.bodies[index].is_some());
}

// A loop whose count is known and small is run pass by pass, so what one pass establishes
// is exactly what the next one sees.

#[test]
fn a_counted_loop_proves_an_exact_total() {
    let sum =
        "f() in 45 to 45:\n  s in 0 to 100: 0\n  for i in 0 to 9 do\n    s :: s + i\n  return s\n";
    assert!(check(sum).is_ok(), "{:?}", check(sum));
    let off_by_one = sum.replace("in 45 to 45", "in 44 to 44");
    assert!(check(&off_by_one).is_err());
}

#[test]
fn a_counter_update_with_a_decided_guard_is_exact() {
    // Eight passes: the guard is decided on every one, so no world is added.
    let source = "f() in 8 to 8:\n  n in 0 to 100: 0\n  for i in 0 to 7 do\n    n :: when n < 100 then n + 1 else 100\n  return n\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
    // Ten passes exceed the limit for a branching body, so only the declared range is known.
    let beyond = "f() in 10 to 10:\n  n in 0 to 100: 0\n  for i in 0 to 9 do\n    n :: when n < 100 then n + 1 else 100\n  return n\n";
    assert!(check(beyond).is_err());
}

#[test]
fn nested_counted_loops_multiply_exactly() {
    let source = "f() in 9 to 9:\n  s in 0 to 100: 0\n  for i in 0 to 2 do\n    for j in 0 to 2 do\n      s :: s + 1\n  return s\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn the_counter_is_the_exact_number_of_each_pass() {
    // Every pass has a different known value, so each is in range or the loop fails.
    let ok = format!("{TAKE}f():\n  for i in 3 to 5 do\n    take(i)\n  return\n");
    assert!(check(&ok).is_ok(), "{:?}", check(&ok));
    let too_far = "take(x in 0 to 4): return\nf():\n  for i in 3 to 5 do\n    take(i)\n  return\n";
    assert!(check(too_far).is_err(), "the last pass is 5");
}

#[test]
fn a_single_pass_may_consume_an_outer_owner_but_a_second_may_not() {
    let once = format!("{RES}f():\n  r: make()\n  for i in 0 to 0 do\n    eat(r)\n  return\n");
    assert!(check(&once).is_ok(), "{:?}", check(&once));
    let twice = format!("{RES}f():\n  r: make()\n  for i in 0 to 1 do\n    eat(r)\n  return\n");
    assert_eq!(check(&twice), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn a_large_or_unknown_count_is_checked_once_instead() {
    // 100 passes is beyond the unroll limit, so the sum cannot be proved exactly.
    let large = "f() in 4950 to 4950:\n  s in 0 to 5000: 0\n  for i in 0 to 99 do\n    s :: s + i\n  return s\n";
    assert!(check(large).is_err());
    // The same total does hold when counted.
    let small =
        "f() in 45 to 45:\n  s in 0 to 5000: 0\n  for i in 0 to 9 do\n    s :: s + i\n  return s\n";
    assert!(check(small).is_ok(), "{:?}", check(small));
    let unknown = "f(n in 0 to 9) in 0 to 9:\n  s in 0 to 9: 0\n  for i in 0 to n do\n    s :: 1\n  return s\n";
    assert!(check(unknown).is_ok(), "{:?}", check(unknown));
}

#[test]
fn a_skipped_returning_range_keeps_its_linear_cleanup_obligation() {
    let source = "\
Token has number in 0 to 9
f(n in 0 to 100) in 0 to 9:
  token := Token(number: 1)
  for i in 1 to n do
    Token(number) := token
    return number
  return 0
";
    assert_eq!(check(source).unwrap_err(), SourceCheckError::Unconsumed);
    let cleanup = source.replace(
        "  return 0\n",
        "  Token(number) := token\n  return number\n",
    );
    assert!(check(&cleanup).is_ok(), "{:?}", check(&cleanup));
    let entered = source.replace("n in 0 to 100", "n in 1 to 100");
    assert!(check(&entered).is_ok(), "{:?}", check(&entered));
}

#[test]
fn a_skipped_returning_range_must_establish_the_mut_postcondition() {
    let source = "\
f(n in 0 to 100, x mut in 0 to 9) where x :: 0:
  for i in 1 to n do
    x :: 0
    return
  return
";
    assert_eq!(check(source).unwrap_err(), SourceCheckError::Unproved);
    let valid = source.replace("\n  return\n", "\n  x :: 0\n  return\n");
    assert!(check(&valid).is_ok(), "{:?}", check(&valid));
}

#[test]
fn returning_range_worlds_keep_their_bound_facts_for_callers() {
    let source = "\
f(n in 0 to 100) in 0 to 1:
  for i in 1 to n do
    return 1
  return n
skipped() in 0 to 0: return f(0)
entered() in 1 to 1: return f(100)
";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn a_branching_body_unrolls_fewer_passes_so_worlds_stay_bounded() {
    // An undecided condition per pass: 8 passes are counted, 20 fall back to one check.
    let eight =
        "f(x in 0 to 9):\n  for i in 0 to 7 do\n    r: when x < 5 then 1 else 2\n  return\n";
    assert!(check(eight).is_ok(), "{:?}", check(eight));
    let twenty =
        "f(x in 0 to 9):\n  for i in 0 to 19 do\n    r: when x < 5 then 1 else 2\n  return\n";
    assert!(check(twenty).is_ok(), "{:?}", check(twenty));
}

#[test]
fn a_body_local_can_be_rebound_on_every_counted_pass() {
    let source = format!("{RES}f():\n  for i in 0 to 3 do\n    r: make()\n    eat(r)\n  return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}
