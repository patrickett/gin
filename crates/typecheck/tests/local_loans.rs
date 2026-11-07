//! Local loans: `v: ref x` / `v: mut x` borrow `x` for as long as the handle `v` is in scope,
//! instead of being a plain alias of `x`.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict loan fixture");
    check_file(&file, Limits::default())
}

const BASE: &str = "\
P has x in 0 to 9
look(p ref P): return
poke(p mut P): return
eat(p P):
  P(x) := p
  return
";

/// `x` is a linear record. `body` runs in a block of its own, so every handle it makes is gone
/// when `x` is eaten afterwards. (`poke` first makes the block start and end in the same state,
/// which a one-sided `when` requires.)
fn go(body: &str) -> String {
    let inner: String = body.lines().map(|line| format!("    {line}\n")).collect();
    format!(
        "{BASE}go(flag in 0 to 1):\n  x: P(x: 3)\n  poke(x)\n  when flag = 0 then\n{inner}  eat(x)\n  return\n"
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
fn the_alias_hole_is_closed() {
    // `v` used to be another name for `x`, so eating `v` ended `x`, and `x` could be changed
    // while `v` was still looked at.
    rejects("v: ref x\neat(v)\n", SourceCheckError::MoveOutOfBorrow);
    rejects(
        "v: ref x\npoke(x)\nlook(v)\n",
        SourceCheckError::BorrowConflict,
    );
    rejects(
        "v: mut x\npoke(x)\npoke(v)\n",
        SourceCheckError::BorrowConflict,
    );
}

#[test]
fn a_live_handle_forbids_moving_or_ending_its_source() {
    for kind in ["ref", "mut"] {
        rejects(
            &format!("v: {kind} x\neat(x)\nlook(v)\n"),
            SourceCheckError::BorrowConflict,
        );
    }
}

#[test]
fn a_live_handle_forbids_writing_its_source() {
    for kind in ["ref", "mut"] {
        let source = go(&format!("v: {kind} x\nx.x :: 4\nlook(v)\n"));
        assert_eq!(
            check(&source),
            Err(SourceCheckError::BorrowConflict),
            "{kind}"
        );
    }
}

#[test]
fn a_live_handle_forbids_lending_its_source_mut() {
    rejects(
        "v: ref x\npoke(x)\nlook(v)\n",
        SourceCheckError::BorrowConflict,
    );
    rejects(
        "v: mut x\npoke(x)\nlook(v)\n",
        SourceCheckError::BorrowConflict,
    );
}

#[test]
fn an_exclusive_handle_forbids_reading_its_source() {
    rejects(
        "v: mut x\nlook(x)\nlook(v)\n",
        SourceCheckError::BorrowConflict,
    );
    // Reading `x` through its own handle is the point of the handle.
    accepts("v: mut x\nlook(v)\npoke(v)\n");
}

#[test]
fn two_exclusive_handles_conflict() {
    rejects(
        "a: mut x\nb: mut x\nlook(a)\nlook(b)\n",
        SourceCheckError::BorrowConflict,
    );
}

#[test]
fn an_exclusive_and_a_shared_handle_conflict_in_either_order() {
    rejects(
        "a: mut x\nb: ref x\nlook(a)\nlook(b)\n",
        SourceCheckError::BorrowConflict,
    );
    rejects(
        "a: ref x\nb: mut x\nlook(a)\nlook(b)\n",
        SourceCheckError::BorrowConflict,
    );
}

#[test]
fn shared_handles_coexist_and_leave_the_source_readable() {
    accepts("a: ref x\nb: ref x\nlook(a)\nlook(b)\nlook(x)\nlook(a)\n");
}

#[test]
fn a_shared_handle_cannot_be_written_through_or_lent_exclusively() {
    assert_eq!(
        check(&go("v: ref x\nv.x :: 4\n")),
        Err(SourceCheckError::Immutable)
    );
    rejects("v: ref x\npoke(v)\n", SourceCheckError::BorrowConflict);
    // An exclusive handle can be written through and lent on.
    let through = go("v: mut x\nv.x :: 4\npoke(v)\nlook(v)\n");
    assert!(check(&through).is_ok(), "{:?}", check(&through));
}

#[test]
fn a_handle_to_a_field_keeps_the_whole_root_from_being_taken_apart() {
    // `look(x.b)` is fine (a different field: see `field_loans.rs`); destructuring `x` while
    // `v` still borrows `x.a` is not.
    let source = format!(
        "{BASE}Pair has\n  a P\n  b P\n\
         go():\n  x: Pair(a: P(x: 1), b: P(x: 2))\n  v: mut x.a\n  look(x.b)\n  poke(v)\n  Pair(a, b) := x\n  eat(a)\n  eat(b)\n  return\n"
    );
    assert_eq!(check(&source), Err(SourceCheckError::BorrowConflict));
}

#[test]
fn the_loan_ends_with_the_scope_that_holds_the_handle() {
    for kind in ["ref", "mut"] {
        let source = format!(
            "{BASE}go(flag in 0 to 1):\n  x: P(x: 3)\n  poke(x)\n  when flag = 0 then\n    v: {kind} x\n    look(v)\n  poke(x)\n  eat(x)\n  return\n"
        );
        assert!(check(&source).is_ok(), "{kind}: {:?}", check(&source));
        // Inside the scope the loan is still held.
        let inside = format!(
            "{BASE}go(flag in 0 to 1):\n  x: P(x: 3)\n  poke(x)\n  when flag = 0 then\n    v: {kind} x\n    poke(x)\n    look(v)\n  eat(x)\n  return\n"
        );
        assert_eq!(
            check(&inside),
            Err(SourceCheckError::BorrowConflict),
            "{kind}"
        );
    }
}

#[test]
fn a_borrow_of_a_local_cannot_be_returned() {
    let escaped = format!("{BASE}bad() P:\n  x: P(x: 1)\n  return ref x\n");
    assert_eq!(check(&escaped), Err(SourceCheckError::BorrowEscape));
    // A handle held in a name is a borrow too; it cannot be moved out as the result.
    let named = format!("{BASE}bad() P:\n  x: P(x: 1)\n  v: ref x\n  return v\n");
    assert!(check(&named).is_err());
}

#[test]
fn a_borrow_cannot_be_stored_where_it_would_outlive_its_source() {
    let source = format!(
        "{BASE}go(flag in 0 to 1):\n  a: P(x: 1)\n  outer: ref a\n  when flag = 0 then\n    inner: P(x: 2)\n    outer :: ref inner\n    eat(inner)\n  eat(a)\n  return\n"
    );
    assert_eq!(check(&source), Err(SourceCheckError::BorrowEscape));
}

#[test]
fn a_borrow_cannot_be_stored_in_a_parameter() {
    let source =
        format!("{BASE}go(slot mut P):\n  x: P(x: 1)\n  slot :: ref x\n  eat(x)\n  return\n");
    assert_eq!(check(&source), Err(SourceCheckError::BorrowEscape));
}

#[test]
fn a_borrowed_parameter_can_be_lent_again_through_a_handle() {
    let source = format!(
        "{BASE}pass(p ref P):\n  v: ref p\n  look(v)\n  look(p)\n  return\nexclusive(p mut P):\n  v: mut p\n  poke(v)\n  return\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
    let shared_to_exclusive = format!("{BASE}bad(p ref P):\n  v: mut p\n  poke(v)\n  return\n");
    assert_eq!(
        check(&shared_to_exclusive),
        Err(SourceCheckError::BorrowConflict)
    );
}

#[test]
fn a_handle_of_a_number_is_a_copy_with_no_owner_to_lend() {
    let source = "\
Stats has count in 0 to 9
count_of(value ref Stats) in 0 to 9:
  seen: ref value.count
  return deref seen
";
    assert!(check(source).is_ok(), "{:?}", check(source));
}
