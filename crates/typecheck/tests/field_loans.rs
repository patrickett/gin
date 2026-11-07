//! Loans are on places, not whole roots: a loan on `x.a` does not block `x.b`, and two places
//! overlap exactly when one path is a prefix of the other.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict loan fixture");
    check_file(&file, Limits::default())
}

const BASE: &str = "\
Cell has n in 0 to 9
Pair has
  a Cell
  b Cell
look(c ref Cell): return
poke(c mut Cell): return
eat(c Cell):
  Cell(n) := c
  return
";

/// `p` is a linear pair. `body` runs in a block of its own, so every handle it makes is gone when
/// `p` is taken apart afterwards.
fn go(body: &str) -> String {
    let inner: String = body.lines().map(|line| format!("    {line}\n")).collect();
    format!(
        "{BASE}go(flag in 0 to 1):\n  p: Pair(a: Cell(n: 1), b: Cell(n: 2))\n  when flag = 0 then\n{inner}  Pair(a, b) := p\n  eat(a)\n  eat(b)\n  return\n"
    )
}

fn accepts(body: &str) {
    let source = go(body);
    assert!(check(&source).is_ok(), "{body}: {:?}", check(&source));
}

fn rejects(body: &str, error: SourceCheckError) {
    assert_eq!(check(&go(body)), Err(error), "{body}");
}

#[test]
fn disjoint_fields_can_be_borrowed_exclusively_at_once() {
    accepts("h: mut p.a\nk: mut p.b\npoke(h)\npoke(k)\n");
}

#[test]
fn a_shared_and_an_exclusive_loan_on_different_fields_coexist() {
    accepts("h: mut p.a\nk: ref p.b\npoke(h)\nlook(k)\n");
    accepts("h: ref p.a\nk: mut p.b\nlook(h)\npoke(k)\n");
}

#[test]
fn a_call_can_lend_one_field_while_another_is_borrowed() {
    accepts("h: mut p.a\nlook(p.b)\npoke(h)\n");
    accepts("h: ref p.a\npoke(p.b)\nlook(h)\n");
}

#[test]
fn overlapping_places_still_conflict() {
    // The same field.
    rejects("h: mut p.a\nk: ref p.a\npoke(h)\nlook(k)\n", SourceCheckError::BorrowConflict);
    rejects("h: mut p.a\nk: mut p.a\npoke(h)\npoke(k)\n", SourceCheckError::BorrowConflict);
    rejects("h: ref p.a\nk: mut p.a\nlook(h)\npoke(k)\n", SourceCheckError::BorrowConflict);
}

#[test]
fn a_field_overlaps_the_whole_and_the_parts_inside_it() {
    // The whole against a part, either way round.
    rejects("h: mut p\nk: ref p.b\npoke(h)\nlook(k)\n", SourceCheckError::BorrowConflict);
    rejects("h: mut p.b\nk: ref p\npoke(h)\nlook(k)\n", SourceCheckError::BorrowConflict);
    // A part inside a field.
    rejects("h: mut p.a\nk: ref p.a.n\npoke(h)\nlook(k)\n", SourceCheckError::BorrowConflict);
    rejects("h: mut p.a.n\nk: ref p.a\npoke(h)\nlook(k)\n", SourceCheckError::BorrowConflict);
}

#[test]
fn a_field_loan_still_forbids_moving_the_whole() {
    rejects("h: ref p.a\nPair(a, b) := p\nlook(h)\n", SourceCheckError::BorrowConflict);
    rejects("h: mut p.a\nPair(a, b) := p\npoke(h)\n", SourceCheckError::BorrowConflict);
}

#[test]
fn a_write_to_a_field_is_blocked_only_by_a_loan_that_overlaps_it() {
    accepts("h: ref p.a\np.b.n :: 4\nlook(h)\n");
    rejects("h: ref p.a\np.a.n :: 4\nlook(h)\n", SourceCheckError::BorrowConflict);
}

#[test]
fn reading_a_field_is_blocked_only_by_an_exclusive_loan_that_overlaps_it() {
    accepts("h: mut p.a\nlook(p.b)\npoke(h)\n");
    rejects("h: mut p.a\nlook(p.a)\npoke(h)\n", SourceCheckError::BorrowConflict);
}

#[test]
fn a_loan_ends_when_its_handle_is_gone() {
    // No later use of `h`, and it is out of scope with the block: the field is free again.
    accepts("h: mut p.a\npoke(h)\n");
    accepts("poke(p.a)\npoke(p.a)\n");
}

const VIEWS: &str = "\
Cell has n in 0 to 9
Pair has
  a Cell
  b Cell
View has
  data ref Cell
  k in 0 to 3
MutView has
  data mut Cell
look(c ref Cell): return
poke(c mut Cell): return
eat(c Cell):
  Cell(n) := c
  return
";

fn views(body: &str) -> String {
    let inner: String = body.lines().map(|line| format!("    {line}\n")).collect();
    format!(
        "{VIEWS}go(flag in 0 to 1):\n  p: Pair(a: Cell(n: 1), b: Cell(n: 2))\n  when flag = 0 then\n{inner}  Pair(a, b) := p\n  eat(a)\n  eat(b)\n  return\n"
    )
}

#[test]
fn views_over_different_fields_coexist() {
    let source = views("v: MutView(data: mut p.a)\nw: MutView(data: mut p.b)\npoke(v.data)\npoke(w.data)\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn views_over_the_same_field_conflict() {
    let source = views("v: MutView(data: mut p.a)\nw: View(data: ref p.a, k: 1)\npoke(v.data)\nlook(w.data)\n");
    assert_eq!(check(&source), Err(SourceCheckError::BorrowConflict));
}

#[test]
fn a_borrow_field_takes_an_explicit_borrow_even_of_a_borrowed_parameter() {
    let base = format!("{VIEWS}");
    // Written: accepted.
    let written = format!("{base}view(c ref Cell) View:\n  return View(data: ref c, k: 1)\n");
    assert!(check(&written).is_ok(), "{:?}", check(&written));
    // Left implicit: refused, for a `ref` and for a `mut` parameter alike.
    let implicit = format!("{base}view(c ref Cell) View:\n  return View(data: c, k: 1)\n");
    assert_eq!(check(&implicit), Err(SourceCheckError::Type));
    let implicit = format!("{base}view(c mut Cell) MutView:\n  return MutView(data: c)\n");
    assert_eq!(check(&implicit), Err(SourceCheckError::Type));
    // Neither does a value (not a borrowed parameter) become a borrow without being written.
    let value = format!("{base}view(c Cell) View:\n  return View(data: c, k: 1)\n");
    assert_eq!(check(&value), Err(SourceCheckError::Type));
}

#[test]
fn a_dynamic_length_array_is_only_held_behind_a_borrow() {
    let owned = "Holder has\n  len in 0 to 9\n  data [Cell; len]\nCell has n in 0 to 9\n";
    assert_eq!(check(owned), Err(SourceCheckError::Type));
    for kind in ["ref", "mut"] {
        let borrowed =
            format!("Holder has\n  len in 0 to 9\n  data {kind} [Cell; len]\nCell has n in 0 to 9\n");
        assert!(check(&borrowed).is_ok(), "{kind}: {:?}", check(&borrowed));
    }
    // A fixed-size array is still owned.
    assert!(check("Fixed has\n  data [Cell; 4]\nCell has n in 0 to 9\n").is_ok());
}
