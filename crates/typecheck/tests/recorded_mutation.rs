//! Reassignment and counted loops reach the recorded body as plain dataflow: a local names
//! the node of its latest value, so what the body computes can be evaluated and lowered.
use i256::I256;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceCallee, CheckedSourceFile, Evaluated, Limits,
    SourceCheckError, check_file, evaluate_value,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict recording fixture");
    check_file(&file, Limits::default())
}

struct Bodies<'a>(&'a [Option<CheckedSourceBody>]);

impl BodyResolver for Bodies<'_> {
    fn body(&self, callee: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        match callee {
            CheckedSourceCallee::Local { function } => self.0[function as usize].as_ref(),
            CheckedSourceCallee::Dependency { .. } => None,
        }
    }
}

fn int(value: i64) -> Evaluated {
    Evaluated::Integer(I256::from(value))
}

/// Check `source`, then evaluate the recorded body of `function`; `None` if unrecorded.
fn run(source: &str, function: &str, arguments: &[Evaluated]) -> Option<Evaluated> {
    let checked = check(source).unwrap_or_else(|error| panic!("{error:?}\n{source}"));
    let index = checked.functions.iter().position(|f| f.name == function)?;
    let body = checked.bodies[index].as_ref()?;
    evaluate_value(body, arguments, &Bodies(&checked.bodies), &mut 100_000)
}

#[test]
fn a_reassigned_local_evaluates_to_its_latest_value() {
    let source = "f() in 0 to 9:\n  n in 0 to 9: 1\n  n :: 4\n  n :: n + 3\n  return n\n";
    assert_eq!(run(source, "f", &[]), Some(int(7)));
}

#[test]
fn compound_assignment_evaluates_like_the_expanded_form() {
    let source = "f(a in 0 to 9) in 0 to 99:\n  n in 0 to 99: a\n  n +: 4\n  n *: 3\n  n -: 2\n  return n\n";
    for a in [0, 1, 5, 9] {
        assert_eq!(run(source, "f", &[int(a)]), Some(int((a + 4) * 3 - 2)), "{a}");
    }
}

#[test]
fn a_field_assignment_rebuilds_the_record() {
    let record = "#unrestricted\nP has\n  x in 0 to 9\n  y in 0 to 9\n";
    let source = format!(
        "{record}f(a in 0 to 9) in 0 to 99:\n  q: P(x: a, y: 2)\n  q.x :: 5\n  q.y +: 3\n  return q.x * 10 + q.y\n"
    );
    for a in [0, 9] {
        assert_eq!(run(&source, "f", &[int(a)]), Some(int(55)), "{a}");
    }
}

#[test]
fn an_assignment_to_a_nominal_local_converts_like_a_binding() {
    let source = "\
#unrestricted, #wrap
Level is in 0 to 9
Level has
  get(self) in 0 to 9:
    a in Self: self
    return a
f(a in 0 to 9) in 0 to 9:
  l Level: 1
  l :: a + 5
  return l.get
";
    assert_eq!(run(source, "f", &[int(8)]), Some(int(3)));
    assert_eq!(run(source, "f", &[int(2)]), Some(int(7)));
}

#[test]
fn a_counted_loop_is_unrolled_into_the_recorded_body() {
    let sum = "f() in 0 to 99:\n  s in 0 to 99: 0\n  for i in 1 to 5 do\n    s +: i\n  return s\n";
    assert_eq!(run(sum, "f", &[]), Some(int(15)));
    // The counter is a different constant on each pass, and reaches nested passes too.
    let nested = "f() in 0 to 99:\n  s in 0 to 99: 0\n  for i in 1 to 3 do\n    for j in 1 to 3 do\n      s +: i * j\n  return s\n";
    assert_eq!(run(nested, "f", &[]), Some(int(36)));
}

#[test]
fn a_loop_body_sees_the_parameters_and_may_branch() {
    let source = "f(a in 0 to 9) in 0 to 40:\n  s in 0 to 40: 0\n  for i in 0 to 3 do\n    s :: when i < a then s + 10 else s\n  return s\n";
    for (a, expected) in [(0, 0), (1, 10), (3, 30), (9, 40)] {
        assert_eq!(run(source, "f", &[int(a)]), Some(int(expected)), "{a}");
    }
}

#[test]
fn a_statically_empty_loop_records_nothing() {
    let source = "f() in 4 to 4:\n  n in 0 to 9: 4\n  for i in 5 to 1 do\n    n :: 0\n  return n\n";
    assert_eq!(run(source, "f", &[]), Some(int(4)));
}

#[test]
fn a_loop_of_unknown_count_carries_what_it_assigns() {
    // The count is a parameter, so the body is recorded once and run per pass.
    let sum = "f(n in 0 to 9) in 0 to 99:\n  s in 0 to 99: 0\n  for i in 1 to n do\n    s +: i\n  return s\n";
    for n in 0..=9i64 {
        assert_eq!(run(sum, "f", &[int(n)]), Some(int(n * (n + 1) / 2)), "{n}");
    }
}

#[test]
fn a_loop_can_carry_several_values_and_branch() {
    // Past the checker's unroll limit the body is checked once, so each update is guarded.
    let source = "f(n in 0 to 20) in 0 to 800:\n  a in 0 to 400: 0\n  b in 0 to 400: 1\n  for i in 0 to n do\n    a :: when a < 100 then a + 2 else a\n    b :: when b < 380 then b + i else b\n  return a + b\n";
    for n in 0..=20i64 {
        let passes = n + 1;
        assert_eq!(
            run(source, "f", &[int(n)]),
            Some(int(2 * passes + 1 + n * (n + 1) / 2)),
            "{n}"
        );
    }
}

#[test]
fn a_loop_may_run_a_loop_of_its_own() {
    let source = "f(n in 0 to 4) in 0 to 400:\n  s in 0 to 400: 0\n  for i in 1 to n do\n    for j in 1 to i do\n      s +: 1\n  return s\n";
    for n in 0..=4i64 {
        assert_eq!(run(source, "f", &[int(n)]), Some(int(n * (n + 1) / 2)), "{n}");
    }
}

#[test]
fn a_count_past_the_unroll_limit_is_a_loop_too() {
    let source = "f() in 0 to 999:\n  s in 0 to 999: 0\n  for i in 0 to 99 do\n    s :: when s < 500 then s + 5 else s\n  return s\n";
    assert_eq!(run(source, "f", &[]), Some(int(500)));
}

const BUMP: &str = "\
bump(x mut in 0 to 9, by in 0 to 3) in 0 to 9:
  x :: when x < 6 then x + by else x
  return by
";

#[test]
fn a_mut_number_is_handed_back_to_the_local_it_was_lent_from() {
    let source = format!(
        "{BUMP}f(a in 0 to 6) in 0 to 18:\n  n in 0 to 9: a\n  r: bump(n, 2)\n  bump(n, 3)\n  return n + r\n"
    );
    for a in 0..=6i64 {
        let once = if a < 6 { a + 2 } else { a };
        let twice = if once < 6 { once + 3 } else { once };
        assert_eq!(run(&source, "f", &[int(a)]), Some(int(twice + 2)), "{a}");
    }
}

#[test]
fn a_function_that_changes_a_number_and_returns_nothing() {
    let source = "\
reset(x mut in 0 to 9):
  x :: 0
  return
f(a in 0 to 9) in 0 to 9:
  n in 0 to 9: a
  reset(n)
  return n
";
    for a in [0, 4, 9] {
        assert_eq!(run(source, "f", &[int(a)]), Some(int(0)), "{a}");
    }
}

#[test]
fn a_mut_call_inside_a_loop_is_carried_like_an_assignment() {
    let source = format!(
        "{BUMP}f(count in 0 to 12) in 0 to 9:\n  n in 0 to 9: 0\n  for i in 1 to count do\n    bump(n, 1)\n  return n\n"
    );
    for count in 0..=12i64 {
        assert_eq!(
            run(&source, "f", &[int(count)]),
            Some(int(count.min(6))),
            "{count}"
        );
    }
}

#[test]
fn what_a_callee_does_to_a_mut_number_is_known_when_the_arguments_are() {
    // Every input is known, so the whole function is a constant at compile time. (The
    // checker itself forgets `n` after the call: only the evaluator sees what bump did.)
    let source = format!("{BUMP}f() in 0 to 9:\n  n in 0 to 9: 1\n  bump(n, 3)\n  return n\n");
    assert_eq!(run(&source, "f", &[]), Some(int(4)));
}

#[test]
fn what_the_recorder_cannot_follow_stays_unrecorded() {
    // A `mut` record is not handed back by register yet.
    let borrowed = "#unrestricted\nP has x in 0 to 9\nf(p mut P):\n  p.x :: 3\n  return\n";
    assert_eq!(run(borrowed, "f", &[int(1)]), None);
    // A loop whose condition joins two comparisons is not recorded. (A loop that changes a
    // record is: it carries the record as its registers.)
    let joined = "f(n in 0 to 3) in 0 to 9:\n  i in 0 to 9: 0\n  while i < n and i < 2 do\n    i :: i + 1\n  return i\n";
    assert_eq!(run(joined, "f", &[int(1)]), None);
}

#[test]
fn a_mut_record_is_handed_back_as_a_whole() {
    let source = "\
#unrestricted
P has
  x in 0 to 9
  y in 0 to 9
shift(p mut P):
  p.x :: p.y
  return
f(a in 0 to 9, b in 0 to 9) in 0 to 9:
  q: P(x: a, y: b)
  shift(q)
  return q.x
";
    for (a, b) in [(1, 2), (0, 9), (7, 3)] {
        assert_eq!(run(source, "f", &[int(a), int(b)]), Some(int(b)), "{a} {b}");
    }
}

#[test]
fn a_field_of_a_record_can_be_the_place_lent_mut() {
    let source = "\
#unrestricted
P has
  x in 0 to 9
  y in 0 to 9
clear(n mut in 0 to 9):
  n :: 0
  return
f(a in 0 to 9, b in 0 to 9) in 0 to 18:
  q: P(x: a, y: b)
  clear(q.x)
  return q.x + q.y
";
    for (a, b) in [(1, 2), (9, 9), (0, 5)] {
        assert_eq!(run(source, "f", &[int(a), int(b)]), Some(int(b)), "{a} {b}");
    }
}
