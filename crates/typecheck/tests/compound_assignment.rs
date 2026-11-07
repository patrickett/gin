//! `place +: value` stores `place + value` back, by the same rules as `place + value`.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict compound fixture");
    check_file(&file, Limits::default())
}

#[test]
fn an_integer_local_is_updated_exactly() {
    let source = |range: &str| {
        format!("f() in {range}:\n  n in 0 to 20: 1\n  n +: 4\n  n *: 2\n  n -: 3\n  return n\n")
    };
    assert!(check(&source("7 to 7")).is_ok(), "{:?}", check(&source("7 to 7")));
    assert_eq!(check(&source("6 to 6")), Err(SourceCheckError::Type));
}

#[test]
fn the_result_must_fit_the_places_declared_range() {
    let source = "f() in 0 to 5:\n  n in 0 to 5: 3\n  n +: 3\n  return n\n";
    assert!(check(source).is_err(), "3 + 3 leaves 0 to 5");
    let fits = source.replace("n +: 3", "n +: 2");
    assert!(check(&fits).is_ok(), "{:?}", check(&fits));
}

#[test]
fn a_place_must_be_assignable_like_any_reassignment() {
    assert_eq!(
        check("f(x in 0 to 5) in 0 to 9:\n  x +: 1\n  return x\n"),
        Err(SourceCheckError::Immutable)
    );
    assert_eq!(
        check("f():\n  missing +: 1\n  return\n"),
        Err(SourceCheckError::UnknownName)
    );
    assert!(check("f(x mut in 0 to 5):\n  x +: 0\n  return\n").is_ok());
}

#[test]
fn a_field_path_can_be_updated() {
    let record = "#unrestricted\nP has x in 0 to 9\n";
    let source = format!("{record}f() in 5 to 5:\n  q: P(x: 2)\n  q.x +: 3\n  return q.x\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
    let over = source.replace("q.x +: 3", "q.x +: 8");
    assert!(check(&over).is_err(), "2 + 8 leaves 0 to 9");
}

#[test]
fn an_accumulator_in_a_loop_is_exact() {
    let source = "f() in 6 to 6:\n  s in 0 to 9: 0\n  for i in 1 to 3 do\n    s +: i\n  return s\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn a_nominal_place_uses_its_infix_method() {
    let level = "\
Numeric has
  #infix('+')
  add(self, rhs Self):
    a in Self: self
    b in Self: rhs
    return a + b
#unrestricted, #wrap
Level is in 0 to 9
Level has Numeric
";
    let source = format!("{level}f():\n  a Level: 7\n  a +: 5\n  return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
    // Without an infix method for the operator there is nothing to call.
    let missing = source.replace("a +: 5", "a *: 5");
    assert_eq!(check(&missing), Err(SourceCheckError::Type));
}

#[test]
fn a_linear_place_is_never_overwritten() {
    let source = "Box has n in 0 to 9\nf(b mut Box):\n  b +: 1\n  return\n";
    assert_eq!(check(source), Err(SourceCheckError::OverwriteLinear));
}

#[test]
fn comparisons_have_no_compound_form() {
    assert!(parser::parse_strict("f(x mut in 0 to 5):\n  x <: 1\n  return\n").is_err());
}
