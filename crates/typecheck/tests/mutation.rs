//! `place :: value`: reassignment of `mut` parameters and `:` locals, and their fields.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict mutation fixture");
    check_file(&file, Limits::default())
}

#[test]
fn a_colon_local_can_be_reassigned_within_its_declared_type() {
    let source = "f() in 0 to 5:\n  n in 0 to 5: 1\n  n :: 4\n  return n\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
    let too_big = "f() in 0 to 5:\n  n in 0 to 5: 1\n  n :: 9\n  return n\n";
    assert!(check(too_big).is_err());
}

#[test]
fn the_new_value_replaces_the_old_in_what_the_checker_knows() {
    let source = "f() in 4 to 4:\n  n in 0 to 5: 1\n  n :: 4\n  return n\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
    let stale = "f() in 1 to 1:\n  n in 0 to 5: 1\n  n :: 4\n  return n\n";
    assert!(check(stale).is_err(), "the old value must be forgotten");
}

#[test]
fn a_local_without_a_declared_range_keeps_the_type_of_its_initializer() {
    let source = "f() in 0 to 5:\n  n: 1\n  n :: 2\n  return n\n";
    assert!(check(source).is_err());
}

#[test]
fn only_mut_parameters_and_colon_locals_are_assignable() {
    for source in [
        "f() in 0 to 5:\n  n in 0 to 5 := 1\n  n :: 2\n  return n\n",
        "f(x in 0 to 5) in 0 to 5:\n  x :: 1\n  return x\n",
        "f(x ref in 0 to 5) in 0 to 5:\n  x :: 1\n  return x\n",
    ] {
        assert_eq!(check(source), Err(SourceCheckError::Immutable), "{source}");
    }
    let mutable = "f(x mut in 0 to 5):\n  x :: 1\n  return\n";
    assert!(check(mutable).is_ok(), "{:?}", check(mutable));
}

#[test]
fn an_unknown_name_cannot_be_assigned() {
    assert_eq!(
        check("f():\n  missing :: 1\n  return\n"),
        Err(SourceCheckError::UnknownName)
    );
}

const P: &str = "P has\n  x in 0 to 9\n  y in 0 to 9\n";

#[test]
fn a_field_of_a_mut_record_can_be_assigned() {
    let source = format!("{P}reset(p mut P):\n  p.x :: 0\n  return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn a_field_of_a_ref_or_owned_record_cannot_be_assigned() {
    for header in ["p ref P", "p P"] {
        let source = format!("{P}reset({header}):\n  p.x :: 0\n  return\n");
        assert_eq!(check(&source), Err(SourceCheckError::Immutable), "{header}");
    }
}

#[test]
fn a_field_assignment_updates_what_is_known_of_that_field_only() {
    let source =
        format!("{P}f() in 5 to 5:\n  p: P(x: 1, y: 2)\n  p.x :: 5\n  P(x, y) := p\n  return x\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
    let other =
        format!("{P}f() in 2 to 2:\n  p: P(x: 1, y: 2)\n  p.x :: 5\n  P(x, y) := p\n  return y\n");
    assert!(check(&other).is_ok(), "{:?}", check(&other));
}

#[test]
fn a_field_assignment_must_fit_the_fields_type() {
    let source = format!("{P}f(p mut P):\n  p.x :: 20\n  return\n");
    assert!(check(&source).is_err());
    let unknown = format!("{P}f(p mut P):\n  p.z :: 1\n  return\n");
    assert_eq!(check(&unknown), Err(SourceCheckError::UnknownName));
    let through_scalar = format!("{P}f(p mut P):\n  p.x.y :: 1\n  return\n");
    assert_eq!(check(&through_scalar), Err(SourceCheckError::Type));
}

#[test]
fn a_field_path_can_be_assigned_through_nested_records() {
    let source = "\
Inner has v in 0 to 9
Outer has
  a Inner
  b in 0 to 9
set(o mut Outer):
  o.a.v :: 3
  o.b :: 4
  return
";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

const RES: &str = "\
Res has v in 0 to 9
Outer has
  inner Res
  n in 0 to 9
make() Res: return Res(v: 1)
";

#[test]
fn a_linear_value_cannot_be_overwritten() {
    let field = format!("{RES}f(o mut Outer):\n  o.inner :: make()\n  return\n");
    assert_eq!(check(&field), Err(SourceCheckError::OverwriteLinear));
    let local = format!("{RES}f():\n  r: make()\n  r :: make()\n  Res(v) := r\n  return\n");
    assert_eq!(check(&local), Err(SourceCheckError::OverwriteLinear));
    // The plain number beside the linear field is still assignable.
    let number = format!("{RES}f(o mut Outer):\n  o.n :: 3\n  return\n");
    assert!(check(&number).is_ok(), "{:?}", check(&number));
}

const BOUNDS: &str = "\
Bounds has
  lower in 0 to 16
  upper in 0 to 16 and >= lower
";

#[test]
fn an_assignment_must_keep_the_records_own_constraints() {
    let broken = format!("{BOUNDS}f(b mut Bounds):\n  b.lower :: 10\n  return\n");
    assert!(check(&broken).is_err(), "upper may be below 10");
    let kept = format!("{BOUNDS}f(b mut Bounds):\n  b.lower :: 0\n  return\n");
    assert!(check(&kept).is_ok(), "{:?}", check(&kept));
    let proved = format!("{BOUNDS}f(b mut Bounds):\n  b.upper :: b.lower\n  return\n");
    assert!(check(&proved).is_ok(), "{:?}", check(&proved));
}

#[test]
fn a_copy_of_an_unrestricted_record_is_not_changed_by_writing_to_the_original() {
    let record = "#unrestricted\nP has x in 0 to 9\n";
    let copy_kept =
        format!("{record}f() in 1 to 1:\n  a: P(x: 1)\n  b: a\n  a.x :: 5\n  return b.x\n");
    assert!(check(&copy_kept).is_ok(), "{:?}", check(&copy_kept));
    let original_changed =
        format!("{record}f() in 5 to 5:\n  a: P(x: 1)\n  b: a\n  a.x :: 5\n  return a.x\n");
    assert!(
        check(&original_changed).is_ok(),
        "{:?}",
        check(&original_changed)
    );
    let wrong = format!("{record}f() in 1 to 1:\n  a: P(x: 1)\n  b: a\n  a.x :: 5\n  return a.x\n");
    assert!(check(&wrong).is_err());
}

#[test]
fn a_call_that_borrows_mut_makes_the_caller_forget_the_place() {
    let source = format!("{P}poke(p mut P):\n  p.x :: 3\n  return\n");
    let known = format!(
        "{source}f() in 0 to 9:\n  p: P(x: 1, y: 2)\n  poke(p)\n  P(x, y) := p\n  return x\n"
    );
    assert!(check(&known).is_ok(), "{:?}", check(&known));
    let assumed = format!(
        "{source}f() in 1 to 1:\n  p: P(x: 1, y: 2)\n  poke(p)\n  P(x, y) := p\n  return x\n"
    );
    assert!(check(&assumed).is_err(), "x may have changed");
}

#[test]
fn a_scalar_lent_mut_is_forgotten_too() {
    let source = "\
set(x mut in 0 to 9):
  x :: 3
  return
f() in 0 to 9:
  n in 0 to 9: 1
  set(n)
  return n
";
    assert!(check(source).is_ok(), "{:?}", check(source));
    let assumed = source.replace("f() in 0 to 9", "f() in 1 to 1");
    assert!(check(&assumed).is_err());
}

#[test]
fn a_shared_borrow_does_not_make_the_caller_forget() {
    let source = format!(
        "{P}look(p ref P): return\nf() in 1 to 1:\n  p: P(x: 1, y: 2)\n  look(p)\n  P(x, y) := p\n  return x\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn a_mut_receiver_can_assign_and_the_caller_forgets() {
    let methods = "\
C has
  n in 0 to 9
  set(self mut):
    self.n :: 7
    return
  eat(self):
    C(n) := self
    return
";
    let source = format!("{methods}go():\n  c: C(n: 1)\n  c.set\n  c.eat\n  return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn the_value_may_be_a_conditional_whose_branches_narrow() {
    let source =
        "f() in 0 to 9:\n  x in 0 to 9: 4\n  x :: when x < 9 then x + 1 else 9\n  return x\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn the_hidden_number_of_a_domain_cannot_be_assigned() {
    let source = "FileId is in 0 to 255\nFileId has\n  close(self): return\nf(id mut FileId):\n  id.value :: 1\n  return\n";
    assert_eq!(check(source), Err(SourceCheckError::Privacy));
}

#[test]
fn a_body_that_mutates_a_mut_record_is_recorded_as_handing_it_back() {
    let source = format!("{P}reset(p mut P):\n  p.x :: 0\n  return\n");
    let checked = check(&source).unwrap();
    let index = checked
        .functions
        .iter()
        .position(|f| f.name == "reset")
        .unwrap();
    let body = checked.bodies[index].as_ref().expect("recorded");
    assert_eq!(body.outputs.len(), 1);
}

#[test]
fn a_mut_call_changes_only_the_place_it_was_given_not_copies_of_it() {
    let record = "#unrestricted\nP has x in 0 to 9\npoke(p mut P):\n  p.x :: 3\n  return\n";
    let copy_kept =
        format!("{record}f() in 1 to 1:\n  a: P(x: 1)\n  b: a\n  poke(a)\n  return b.x\n");
    assert!(check(&copy_kept).is_ok(), "{:?}", check(&copy_kept));
    let original_forgotten =
        format!("{record}f() in 1 to 1:\n  a: P(x: 1)\n  b: a\n  poke(a)\n  return a.x\n");
    assert!(check(&original_forgotten).is_err());
}

#[test]
fn a_field_lent_mut_is_forgotten_but_its_siblings_are_not() {
    let source = "\
#unrestricted
Inner has v in 0 to 9
#unrestricted
Outer has
  a Inner
  n in 0 to 9
poke(i mut Inner):
  i.v :: 3
  return
";
    let sibling = format!(
        "{source}f() in 7 to 7:\n  o: Outer(a: Inner(v: 1), n: 7)\n  poke(o.a)\n  return o.n\n"
    );
    assert!(check(&sibling).is_ok(), "{:?}", check(&sibling));
    let lent = format!(
        "{source}f() in 1 to 1:\n  o: Outer(a: Inner(v: 1), n: 7)\n  poke(o.a)\n  return o.a.v\n"
    );
    assert!(check(&lent).is_err());
}
