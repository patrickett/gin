//! Code that can never run is still checked, and reported as unreachable.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, SourceCheckNoteKind, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict fixture");
    check_file(&file, Limits::default())
}

fn unreachable_notes(checked: &CheckedSourceFile) -> usize {
    checked
        .notes
        .iter()
        .filter(|note| note.kind == SourceCheckNoteKind::UnreachableCode)
        .count()
}

#[test]
fn a_loop_over_an_empty_range_is_reported() {
    let source = "f(m in 5 to 9, n in 0 to 3):\n  for i in m to n do\n    r: 1\n  return\n";
    let checked = check(source).unwrap();
    assert_eq!(unreachable_notes(&checked), 1);
    // A loop that can run is not reported.
    let live = "f(m in 0 to 9, n in 0 to 3):\n  for i in m to n do\n    r: 1\n  return\n";
    assert_eq!(unreachable_notes(&check(live).unwrap()), 0);
}

#[test]
fn a_mistake_in_a_dead_loop_body_is_still_an_error() {
    let source = "f(m in 5 to 9, n in 0 to 3):\n  for i in m to n do\n    missing()\n  return\n";
    assert_eq!(check(source), Err(SourceCheckError::UnknownName));
}

#[test]
fn what_a_dead_body_would_have_to_prove_holds_vacuously() {
    // `need` wants a number below 4; in a loop that never runs nothing is wrong.
    let source = "need(x in 0 to 3): return\nf(m in 5 to 9, n in 0 to 3):\n  for i in m to n do\n    need(i)\n  return\n";
    let checked = check(source).unwrap();
    assert_eq!(unreachable_notes(&checked), 1);
}

#[test]
fn a_branch_the_condition_rules_out_is_reported_and_checked() {
    let source = "f(x in 0 to 9) in 0 to 9:\n  return when x < 0 then 1 else x\n";
    assert_eq!(unreachable_notes(&check(source).unwrap()), 1);
    let always = "f(x in 0 to 9) in 0 to 9:\n  return when x >= 0 then x else 1\n";
    assert_eq!(unreachable_notes(&check(always).unwrap()), 1);
    let typo = "f(x in 0 to 9) in 0 to 9:\n  return when x < 0 then missing() else x\n";
    assert_eq!(check(typo), Err(SourceCheckError::UnknownName));
    let live = "f(x in 0 to 9) in 0 to 9:\n  return when x < 5 then 1 else x\n";
    assert_eq!(unreachable_notes(&check(live).unwrap()), 0);
}

#[test]
fn a_dead_branch_does_not_change_the_live_one() {
    let source = "f(x in 0 to 9) in 4 to 4:\n  return when x < 0 then 1 else 4\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
}
