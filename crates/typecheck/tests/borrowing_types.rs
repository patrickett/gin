//! Borrowing types: a record with a `ref`/`mut` field is a view. It is built from a place, owns
//! nothing, and keeps that place borrowed for as long as the view is reachable.
use typecheck::{Borrowing, CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict view fixture");
    check_file(&file, Limits::default())
}

/// `Cell` is linear; `Free` is `#unrestricted`. `View` borrows a cell shared, `MutView` exclusively.
const BASE: &str = "\
Cell has x in 0 to 9
#unrestricted
Free has x in 0 to 9
View has
  data ref Cell
  n in 0 to 3
MutView has
  data mut Cell
FreeView has
  data ref Free
  n in 0 to 3
look(c ref Cell): return
poke(c mut Cell): return
eat(c Cell):
  Cell(x) := c
  return
";

/// `c` is a linear cell; `body` runs in a block of its own so its views are gone afterwards.
fn go(body: &str) -> String {
    let inner: String = body.lines().map(|line| format!("    {line}\n")).collect();
    format!(
        "{BASE}go(flag in 0 to 1):\n  c: Cell(x: 3)\n  poke(c)\n  when flag = 0 then\n{inner}  eat(c)\n  return\n"
    )
}

/// The same, with the block's views made at top level, so they are still held at the end.
fn go_held(body: &str) -> String {
    let inner: String = body.lines().map(|line| format!("  {line}\n")).collect();
    format!("{BASE}go():\n  c: Cell(x: 3)\n{inner}  eat(c)\n  return\n")
}

fn accepts(source: &str) {
    assert!(check(source).is_ok(), "{source}: {:?}", check(source));
}

fn rejects(source: &str, error: SourceCheckError) {
    assert_eq!(check(source), Err(error), "{source}");
}

#[test]
fn the_schema_records_which_fields_borrow() {
    let checked = check(&format!("{BASE}main(): return\n")).expect("checks");
    let borrowing = |name: &str| {
        checked
            .aggregates
            .iter()
            .find(|aggregate| aggregate.name == name)
            .map(|aggregate| aggregate.borrowing)
    };
    assert_eq!(borrowing("Cell"), Some(Borrowing::None));
    assert_eq!(borrowing("View"), Some(Borrowing::Shared));
    assert_eq!(borrowing("MutView"), Some(Borrowing::Exclusive));
    let view = checked
        .aggregates
        .iter()
        .find(|a| a.name == "View")
        .unwrap();
    assert!(view.fields[0].borrow.is_some() && view.fields[1].borrow.is_none());
}

#[test]
fn a_view_is_built_from_a_place_and_dropped_without_release() {
    accepts(&go("v: View(data: ref c, n: 1)\nlook(c)\n"));
    accepts(&go("v: MutView(data: mut c)\n"));
}

#[test]
fn a_view_can_be_built_from_a_handle_held_in_a_local() {
    accepts(&go("h: ref c\nv: View(data: h, n: 2)\nlook(c)\n"));
    accepts(&go("h: mut c\nv: MutView(data: h)\n"));
}

#[test]
fn a_view_can_be_taken_apart_and_its_borrow_used() {
    accepts(&go(
        "v: View(data: ref c, n: 1)\nView(d, n) := v\nlook(d)\nlook(c)\n",
    ));
}

#[test]
fn a_ref_only_unrestricted_view_is_copyable() {
    let source = format!(
        "{BASE}#unrestricted\nCopyView has\n  data ref Free\nlook_free(f ref Free): return\n\
         go():\n  f: Free(x: 1)\n  v: CopyView(data: ref f)\n  w: v\n  z: v\n  look_free(f)\n  return\n"
    );
    accepts(&source);
}

#[test]
fn an_unrestricted_view_cannot_hold_an_exclusive_borrow() {
    let source = format!("{BASE}#unrestricted\nBad has\n  data mut Free\n");
    rejects(&source, SourceCheckError::UnrestrictedField);
}

#[test]
fn an_unmarked_view_is_not_copyable_and_a_mut_view_is_moved() {
    // Not `#unrestricted`, so reading it moves it.
    rejects(
        &go("v: View(data: ref c, n: 1)\nw: v\nz: v\n"),
        SourceCheckError::UseAfterMove,
    );
    rejects(
        &go("v: MutView(data: mut c)\nw: v\nz: v\n"),
        SourceCheckError::UseAfterMove,
    );
}

#[test]
fn a_borrow_field_cannot_be_given_a_value() {
    rejects(&go("v: View(data: c, n: 1)\n"), SourceCheckError::Type);
    rejects(
        &go("v: View(data: Cell(x: 1), n: 1)\n"),
        SourceCheckError::Type,
    );
    rejects(&go("v: View(data: 3, n: 1)\n"), SourceCheckError::Type);
}

#[test]
fn a_mut_field_needs_a_mut_borrow() {
    rejects(&go("v: MutView(data: ref c)\n"), SourceCheckError::Type);
    rejects(
        &go("h: ref c\nv: MutView(data: h)\n"),
        SourceCheckError::Type,
    );
    // A mut borrow may be used as shared by a `ref` field.
    accepts(&go("v: View(data: mut c, n: 1)\n"));
    accepts(&go("h: mut c\nv: View(data: h, n: 1)\n"));
}

#[test]
fn a_handle_moves_into_a_view() {
    // The exclusive handle `h` is spent once the view holds it.
    rejects(
        &go("h: mut c\nv: MutView(data: h)\nw: MutView(data: h)\n"),
        SourceCheckError::UseAfterMove,
    );
}

#[test]
fn a_live_view_keeps_its_source_borrowed() {
    rejects(
        &go_held("v: View(data: ref c, n: 1)\neat(c)\nView(d, n) := v\n"),
        SourceCheckError::BorrowConflict,
    );
    rejects(
        &go("v: View(data: ref c, n: 1)\npoke(c)\n"),
        SourceCheckError::BorrowConflict,
    );
    rejects(
        &go("v: View(data: ref c, n: 1)\nc.x :: 4\n"),
        SourceCheckError::BorrowConflict,
    );
    // A mut view also forbids reading the source.
    rejects(
        &go("v: MutView(data: mut c)\nlook(c)\n"),
        SourceCheckError::BorrowConflict,
    );
    accepts(&go("v: View(data: ref c, n: 1)\nlook(c)\n"));
}

#[test]
fn views_of_one_source_follow_the_handle_rules() {
    accepts(&go(
        "a: View(data: ref c, n: 1)\nb: View(data: ref c, n: 2)\nlook(c)\n",
    ));
    rejects(
        &go("a: MutView(data: mut c)\nb: MutView(data: mut c)\n"),
        SourceCheckError::BorrowConflict,
    );
    rejects(
        &go("a: MutView(data: mut c)\nb: View(data: ref c, n: 1)\n"),
        SourceCheckError::BorrowConflict,
    );
    rejects(
        &go("a: View(data: ref c, n: 1)\nb: MutView(data: mut c)\n"),
        SourceCheckError::BorrowConflict,
    );
}

#[test]
fn dropping_the_view_ends_the_loan() {
    accepts(&format!(
        "{BASE}go(flag in 0 to 1):\n  c: Cell(x: 3)\n  poke(c)\n  when flag = 0 then\n    v: MutView(data: mut c)\n  poke(c)\n  eat(c)\n  return\n"
    ));
    // Moving the view into another block's name does not extend the loan past that name.
    accepts(&go("v: View(data: ref c, n: 1)\nView(d, n) := v\n"));
}

#[test]
fn a_view_is_not_written_through_a_ref_field() {
    rejects(
        &go("v: View(data: ref c, n: 1)\nv.data.x :: 1\n"),
        SourceCheckError::Immutable,
    );
    // A mut field may be written through.
    accepts(&go("v: MutView(data: mut c)\nv.data.x :: 1\n"));
}

#[test]
fn a_view_of_a_local_cannot_be_returned() {
    let source =
        format!("{BASE}bad() View:\n  c: Cell(x: 1)\n  v: View(data: ref c, n: 1)\n  return v\n");
    rejects(&source, SourceCheckError::BorrowEscape);
    let direct = format!("{BASE}bad() View:\n  c: Cell(x: 1)\n  return View(data: ref c, n: 1)\n");
    rejects(&direct, SourceCheckError::BorrowEscape);
}

#[test]
fn a_view_of_an_owned_parameter_cannot_be_returned() {
    let source = format!("{BASE}bad(c Cell) View:\n  return View(data: ref c, n: 1)\n");
    rejects(&source, SourceCheckError::BorrowEscape);
}

#[test]
fn a_view_of_a_borrowed_parameter_can_be_returned() {
    let shared = format!("{BASE}view(c ref Cell) View:\n  return View(data: ref c, n: 1)\n");
    accepts(&shared);
    let lent = format!("{BASE}view(c ref Cell) View:\n  h: ref c\n  return View(data: h, n: 1)\n");
    accepts(&lent);
    let exclusive = format!("{BASE}view(c mut Cell) MutView:\n  return MutView(data: mut c)\n");
    accepts(&exclusive);
    // A shared parameter cannot back an exclusive view.
    let wrong = format!("{BASE}view(c ref Cell) MutView:\n  return MutView(data: mut c)\n");
    rejects(&wrong, SourceCheckError::BorrowConflict);
}

#[test]
fn a_borrowed_parameter_is_never_lent_to_a_view_field_implicitly() {
    // The borrow is always written, even for a parameter that is already borrowed.
    let shared = format!("{BASE}view(c ref Cell) View:\n  return View(data: c, n: 1)\n");
    rejects(&shared, SourceCheckError::Type);
    let exclusive = format!("{BASE}view(c mut Cell) MutView:\n  return MutView(data: c)\n");
    rejects(&exclusive, SourceCheckError::Type);
}

#[test]
fn a_view_cannot_be_stored_where_it_outlives_its_source() {
    let source = format!(
        "{BASE}go(flag in 0 to 1):\n  a: Cell(x: 1)\n  outer: View(data: ref a, n: 0)\n  when flag = 0 then\n    inner: Cell(x: 2)\n    outer :: View(data: ref inner, n: 1)\n    eat(inner)\n  View(d, n) := outer\n  eat(a)\n  return\n"
    );
    rejects(&source, SourceCheckError::BorrowEscape);
    let into_parameter = format!(
        "{BASE}go(slot mut View):\n  c: Cell(x: 1)\n  slot :: View(data: ref c, n: 1)\n  eat(c)\n  return\n"
    );
    rejects(&into_parameter, SourceCheckError::BorrowEscape);
}

#[test]
fn a_view_may_be_passed_to_a_function() {
    let source = format!(
        "{BASE}use_view(v View):\n  View(d, n) := v\n  look(d)\n  return\n\
         peek(v ref View): return\n\
         go(flag in 0 to 1):\n  c: Cell(x: 3)\n  poke(c)\n  when flag = 0 then\n    v: View(data: ref c, n: 1)\n    peek(v)\n    use_view(v)\n  eat(c)\n  return\n"
    );
    accepts(&source);
    // A view parameter that is dropped needs no release.
    accepts(&format!("{BASE}drop_view(v View): return\n"));
}

#[test]
fn a_view_may_hold_another_view() {
    let nested = format!(
        "{BASE}Outer has\n  inner View\n  k in 0 to 3\n\
         go(flag in 0 to 1):\n  c: Cell(x: 3)\n  poke(c)\n  when flag = 0 then\n    o: Outer(inner: View(data: ref c, n: 1), k: 2)\n    poke(c)\n  eat(c)\n  return\n"
    );
    // The outer view keeps `c` borrowed through the inner one.
    rejects(&nested, SourceCheckError::BorrowConflict);
    let released = nested.replace("    poke(c)\n  eat", "  eat");
    accepts(&released);
    let escaping = format!(
        "{BASE}Outer has\n  inner View\n  k in 0 to 3\n\
         bad() Outer:\n  c: Cell(x: 1)\n  return Outer(inner: View(data: ref c, n: 1), k: 2)\n"
    );
    rejects(&escaping, SourceCheckError::BorrowEscape);
}

#[test]
fn a_borrowing_record_that_also_owns_something_still_has_to_be_consumed() {
    let source = format!(
        "{BASE}Holder has\n  data ref Cell\n  extra Cell\n\
         go(flag in 0 to 1):\n  c: Cell(x: 3)\n  poke(c)\n  when flag = 0 then\n    h: Holder(data: ref c, extra: Cell(x: 1))\n  eat(c)\n  return\n"
    );
    rejects(&source, SourceCheckError::Unconsumed);
}
