//! Calls whose results borrow from their arguments (stored borrows, slice 4): the borrow is
//! inferred from the verified body, and a result that borrows a local is rejected.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict fixture");
    check_file(&file, Limits::default())
}

fn accepts(source: &str) {
    assert!(check(source).is_ok(), "{source}: {:?}", check(source));
}

fn rejects(source: &str, error: SourceCheckError) {
    assert_eq!(check(source), Err(error), "{source}");
}

const BASE: &str = "\
#unrestricted
Byte is in 0 to 255
Cell has x in 0 to 9
";

#[test]
fn a_ref_result_is_a_borrow_of_a_parameter() {
    accepts(&format!("{BASE}first(c ref Cell) ref Cell: return ref c\nmain(): return\n"));
}

const VIEW: &str = "\
#unrestricted
Length is in 0 to 4096
View(element) has
  len Length
  data ref [element; len]

  borrow(self ref, index Length and index < self.len) ref element:
    return ref self.data.[index]
";

#[test]
fn a_view_lends_one_element() {
    accepts(&format!("{VIEW}main(): return\n"));
}

const HOLDER: &str = "\
Cell has x in 0 to 9
Holder has
  cell Cell
  look(self ref) ref Cell: return ref self.cell
  poke(self mut) ref Cell: return ref self.cell
  first(self ref, other ref Holder) ref Cell: return ref self.cell
  second(self ref, other ref Holder) ref Cell: return ref other.cell
peek(c ref Cell): return
bump(h mut Holder): return
eat(h Holder):
  Holder(cell) := h
  Cell(x) := cell
  return
";

fn go(body: &str) -> String {
    let inner: String = body.lines().map(|line| format!("  {line}\n")).collect();
    format!("{HOLDER}go(flag in 0 to 1):\n  h: Holder(cell: Cell(x: 3))\n  g: Holder(cell: Cell(x: 4))\n{inner}  eat(h)\n  eat(g)\n  return\n")
}

fn scoped(body: &str) -> String {
    let inner: String = body.lines().map(|line| format!("    {line}\n")).collect();
    format!("{HOLDER}go(flag in 0 to 1):\n  h: Holder(cell: Cell(x: 3))\n  g: Holder(cell: Cell(x: 4))\n  when flag = 0 then\n{inner}  eat(h)\n  eat(g)\n  return\n")
}

#[test]
fn a_result_keeps_its_source_borrowed_while_it_is_live() {
    // The result is gone with its block, so the source may be moved afterwards.
    accepts(&scoped("c: h.look\npeek(c)"));
    // While it is in scope the source cannot be moved or lent `mut`.
    rejects(&go("c: h.look\neat(h)\npeek(c)").replace("  eat(h)\n  eat(g)", "  eat(g)"), SourceCheckError::BorrowConflict);
}

#[test]
fn only_the_arguments_the_body_returned_stay_borrowed() {
    // `second` borrows from `other`: `h` is free to change, `g` is not.
    accepts(&scoped("c: h.second(g)\nbump(h)\npeek(c)"));
    rejects(&scoped("c: h.second(g)\nbump(g)\npeek(c)"), SourceCheckError::BorrowConflict);
    // `first` borrows from `self` only.
    accepts(&scoped("c: h.first(g)\nbump(g)\npeek(c)"));
    rejects(&scoped("c: h.first(g)\nbump(h)\npeek(c)"), SourceCheckError::BorrowConflict);
}

#[test]
fn a_live_result_blocks_a_mutable_borrow_of_its_source() {
    rejects(&scoped("c: h.look\nd: h.poke\npeek(c)\npeek(d)"), SourceCheckError::BorrowConflict);
    rejects(&scoped("c: h.look\nbump(h)\npeek(c)"), SourceCheckError::BorrowConflict);
    // Once the result is gone the source is free again, and two shared results coexist.
    accepts(&scoped("c: h.look\nd: h.look\npeek(c)\npeek(d)"));
    accepts(&scoped("bump(h)\nc: h.look\npeek(c)"));
}

#[test]
fn a_result_that_borrows_an_owned_parameter_escapes() {
    rejects(
        &format!("{HOLDER}stolen(h Holder) ref Cell: return ref h.cell\n"),
        SourceCheckError::BorrowEscape,
    );
    rejects(
        &format!("{HOLDER}local() ref Cell:\n  h: Holder(cell: Cell(x: 1))\n  return ref h.cell\n"),
        SourceCheckError::BorrowEscape,
    );
}

#[test]
fn a_result_passes_through_a_wrapper() {
    let wrapper = format!("{HOLDER}via(h ref Holder) ref Cell: return h.look\n");
    accepts(&format!("{wrapper}main(): return\n"));
}

const VIEWS: &str = "\
Cell has x in 0 to 9
View has
  data ref Cell
  look(self ref) ref Cell: return ref self.data
Owner has
  cell Cell
  view(self ref) View: return View(data: ref self.cell)
peek(c ref Cell): return
bump(o mut Owner): return
eat(o Owner):
  Owner(cell) := o
  Cell(x) := cell
  return
";

fn with_owner(body: &str) -> String {
    let inner: String = body.lines().map(|line| format!("    {line}\n")).collect();
    format!("{VIEWS}go(flag in 0 to 1):\n  o: Owner(cell: Cell(x: 3))\n  when flag = 0 then\n{inner}  eat(o)\n  return\n")
}

#[test]
fn a_returned_view_keeps_the_owner_borrowed() {
    accepts(&with_owner("v: o.view\npeek(v.data)"));
    rejects(&with_owner("v: o.view\nbump(o)\npeek(v.data)"), SourceCheckError::BorrowConflict);
    accepts(&with_owner("bump(o)\nv: o.view\npeek(v.data)"));
}

#[test]
fn a_result_borrowed_through_a_view_keeps_what_the_view_borrows() {
    accepts(&with_owner("v: o.view\nc: v.look\npeek(c)"));
    // The result is a copy of the view's own borrow of `o`, so `o` stays borrowed even
    // when the view itself is no longer used.
    rejects(&with_owner("v: o.view\nc: v.look\nbump(o)\npeek(c)"), SourceCheckError::BorrowConflict);
}

const OPAQUE: &str = "\
Cell has x in 0 to 9
Holder has
  cell Cell
#instruction('ldr x0, [x0]')
cell_of(h ref Holder) ref Cell
peek(c ref Cell): return
bump(h mut Holder): return
eat(h Holder):
  Holder(cell) := h
  Cell(x) := cell
  return
";

fn with_opaque(body: &str) -> String {
    let inner: String = body.lines().map(|line| format!("    {line}\n")).collect();
    format!("{OPAQUE}go(flag in 0 to 1):\n  h: Holder(cell: Cell(x: 3))\n  when flag = 0 then\n{inner}  eat(h)\n  return\n")
}

#[test]
fn a_function_without_a_body_borrows_from_every_borrowed_argument() {
    accepts(&with_opaque("c: cell_of(h)\npeek(c)"));
    rejects(&with_opaque("c: cell_of(h)\nbump(h)\npeek(c)"), SourceCheckError::BorrowConflict);
    accepts(&with_opaque("bump(h)\nc: cell_of(h)\npeek(c)"));
}

fn module(
    name: &str,
    text: &str,
    dependencies: &[&typecheck::CheckedSourceModule],
) -> Result<typecheck::CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    typecheck::check_module(name, &source, dependencies, Limits::default())
}

#[test]
fn an_imported_function_keeps_its_result_sources() {
    let library = module(
        "lib.cells",
        "Cell has x in 0 to 9\nHolder has\n  cell Cell\n  look(self ref) ref Cell: return ref self.cell\npeek(c ref Cell): return\nbump(h mut Holder): return\neat(h Holder):\n  Holder(cell) := h\n  Cell(x) := cell\n  return\n",
        &[],
    )
    .unwrap();
    let client = |body: &str| {
        let inner: String = body.lines().map(|line| format!("    {line}\n")).collect();
        module(
            "client",
            &format!("use lib.cells Cell, Holder, peek, bump, eat\ngo(flag in 0 to 1):\n  h: Holder(cell: Cell(x: 3))\n  when flag = 0 then\n{inner}  eat(h)\n  return\n"),
            &[&library],
        )
        .map(|_| ())
    };
    assert_eq!(client("c: h.look\npeek(c)"), Ok(()));
    assert_eq!(client("c: h.look\nbump(h)\npeek(c)"), Err(SourceCheckError::BorrowConflict));
}
