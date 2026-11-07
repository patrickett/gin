//! `where place :: value`: what a call does to the places it is lent `mut`, proved where the
//! function returns and used by callers instead of forgetting the place.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict contract fixture");
    check_file(&file, Limits::default())
}

const RESET: &str = "reset(x mut in 0 to 9) where x :: 0:\n  x :: 0\n  return\n";

#[test]
fn a_caller_knows_what_a_call_left_in_the_place_it_lent() {
    let source = |range: &str| {
        format!("{RESET}f(a in 0 to 9) in {range}:\n  n in 0 to 9: a\n  reset(n)\n  return n\n")
    };
    assert!(check(&source("0 to 0")).is_ok(), "{:?}", check(&source("0 to 0")));
    // The control: without the clause the call is forgotten and the same program is refused.
    let silent = source("0 to 0").replace(" where x :: 0", "");
    assert_eq!(check(&silent), Err(SourceCheckError::Type));
}

#[test]
fn a_body_that_does_not_establish_its_clause_is_refused() {
    let wrong = RESET.replace("  x :: 0\n  return", "  x :: 1\n  return");
    assert_eq!(check(&wrong), Err(SourceCheckError::Unproved));
    let untouched = RESET.replace("  x :: 0\n  return", "  return");
    assert_eq!(check(&untouched), Err(SourceCheckError::Unproved));
}

const INC: &str = "\
inc(x mut in 0 to 9) where x :: when x < 9 then x + 1 else x:
  x :: when x < 9 then x + 1 else x
  return
";

#[test]
fn a_clause_may_say_how_the_new_value_follows_from_the_old() {
    let known = |range: &str| format!("{INC}f() in {range}:\n  n in 0 to 9: 4\n  inc(n)\n  return n\n");
    assert!(check(&known("5 to 5")).is_ok(), "{:?}", check(&known("5 to 5")));
    assert_eq!(check(&known("4 to 4")), Err(SourceCheckError::Type));
    // At the top of the range it saturates, which the caller also knows.
    let top = format!("{INC}f() in 9 to 9:\n  n in 0 to 9: 9\n  inc(n)\n  return n\n");
    assert!(check(&top).is_ok(), "{:?}", check(&top));
}

#[test]
fn an_unknown_start_is_known_by_its_difference() {
    // Whatever `a` is, a call that adds one leaves `n` one above it (when it was below 9).
    let source = format!(
        "{INC}f(a in 0 to 8) in 1 to 1:\n  n in 0 to 9: a\n  inc(n)\n  return n - a\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn what_is_not_provably_that_value_is_refused() {
    // A product is a relational definition too: the same product written again is the same number,
    // so keeping this promise is proved. A different product is refused.
    let same = "same(x mut in 0 to 9) where x :: x * 1:\n  x :: x * 1\n  return\n";
    assert!(check(same).is_ok(), "{:?}", check(same));
    let differs = "same(x mut in 0 to 9) where x :: x * 1:\n  x :: x * 0\n  return\n";
    assert_eq!(check(differs), Err(SourceCheckError::Unproved));
    let wrong = INC.replace("where x :: when x < 9 then x + 1", "where x :: when x < 9 then x + 2");
    assert_eq!(check(&wrong), Err(SourceCheckError::Unproved));
    // Adding a number that is not a constant is now tracked by its definition
    // The promise is kept. A different one is still refused.
    let variable = "add(x mut in 0 to 9, by in 0 to 3) where x :: when x < 6 then x + by else x:\n  x :: when x < 6 then x + by else x\n  return\n";
    assert!(check(variable).is_ok(), "{:?}", check(variable));
    let other = "add(x mut in 0 to 9, by in 0 to 3) where x :: when x < 6 then x + by else x:\n  x :: when x < 6 then x + 1 else x\n  return\n";
    assert_eq!(check(other), Err(SourceCheckError::Unproved));
}

#[test]
fn a_place_the_body_never_writes_is_unchanged_after_the_call() {
    let source = |range: &str| {
        format!(
            "set(x mut in 0 to 9, y mut in 0 to 9):\n  y :: 0\n  return\nf() in {range}:\n  a in 0 to 9: 4\n  b in 0 to 9: 5\n  set(a, b)\n  return a\n"
        )
    };
    assert!(check(&source("4 to 4")).is_ok(), "{:?}", check(&source("4 to 4")));
    // The place it does write, with no clause, is forgotten.
    let forgotten = source("4 to 4").replace("return a", "return b").replace("in 4 to 4", "in 0 to 0");
    assert_eq!(check(&forgotten), Err(SourceCheckError::Type));
}

#[test]
fn record_fields_have_their_own_clauses_and_frame() {
    let record = "#unrestricted\nP has\n  x in 0 to 9\n  y in 0 to 9\nstep(p mut P) where p.x :: p.y:\n  p.x :: p.y\n  return\n";
    let source = |tail: &str, range: &str| {
        format!("{record}f() in {range}:\n  q: P(x: 1, y: 5)\n  step(q)\n  return {tail}\n")
    };
    assert!(check(&source("q.x", "5 to 5")).is_ok(), "{:?}", check(&source("q.x", "5 to 5")));
    // The field the function does not mention is unchanged.
    assert!(check(&source("q.y", "5 to 5")).is_ok(), "{:?}", check(&source("q.y", "5 to 5")));
    assert_eq!(check(&source("q.x", "1 to 1")), Err(SourceCheckError::Type));
}

#[test]
fn a_clause_names_a_mut_parameter_and_nothing_else() {
    assert_eq!(
        check("f(x in 0 to 9) where x :: 0:\n  return\n"),
        Err(SourceCheckError::Immutable)
    );
    assert_eq!(
        check("f(x mut in 0 to 9) where y :: 0:\n  return\n"),
        Err(SourceCheckError::UnknownName)
    );
    assert_eq!(
        check("f(x mut in 0 to 9) where x :: 0, x :: 1:\n  x :: 0\n  return\n"),
        Err(SourceCheckError::Malformed)
    );
}

#[test]
fn a_clause_may_read_the_other_parameters() {
    let source = "copy(dest mut in 0 to 9, src in 0 to 9) where dest :: src:\n  dest :: src\n  return\nf(a in 0 to 9) in 0 to 0:\n  n in 0 to 9: 3\n  copy(n, a)\n  return n - a\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn a_contract_is_taken_through_a_loop_and_a_second_call() {
    let source = format!(
        "{RESET}f(count in 0 to 12) in 0 to 0:\n  n in 0 to 9: 5\n  for i in 1 to count do\n    reset(n)\n  reset(n)\n  return n\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn a_contract_reaches_callers_in_other_files() {
    use typecheck::check_module_files;
    let files: Vec<_> = [
        "reset(x mut in 0 to 9) where x :: 0:\n  x :: 0\n  return\n",
        "f(a in 0 to 9) in 0 to 0:\n  n in 0 to 9: a\n  reset(n)\n  return n\n",
    ]
    .iter()
    .enumerate()
    .map(|(index, text)| parser::parse_strict_file(text, index as u32).unwrap())
    .collect();
    let module = check_module_files("app", &files, &[], Limits::default());
    assert!(module.is_ok(), "{:?}", module.err());
}

#[test]
fn a_local_binding_cannot_carry_a_contract() {
    assert_eq!(
        check("f():\n  g(x mut in 0 to 9) where x :: 0:\n    x :: 0\n    return\n  return\n"),
        Err(SourceCheckError::Unsupported)
    );
}
