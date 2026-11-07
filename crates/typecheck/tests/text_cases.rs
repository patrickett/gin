//! A case over compile-time text: arms are text literals and the last is `else`.
//!
//! Text exists only while compiling, so the compiler picks the arm: the checker checks every arm
//! (it does not know which text), and the recorder, which does, keeps only the chosen one.
use typecheck::{CheckedSourceBodyKind, CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.text", &source, &[], Limits::default())
}

const PICK: &str = "\
pick(name) in 0 to 9: return when name is
    'putchar' then 1
    'labs' then 2
    else 3
";

fn returned_number(module: &CheckedSourceModule, function: &str) -> Option<i32> {
    let file = module.checked();
    let index = file.functions.iter().position(|f| f.name == function)?;
    let body = file.bodies[index].as_ref()?;
    body.expressions.iter().find_map(|e| match &e.kind {
        CheckedSourceBodyKind::Integer(value) => (0..10).find(|n| *value == (*n).into()),
        _ => None,
    })
}

#[test]
fn a_case_over_text_dispatches_at_compile_time() {
    let source = "\
label() : 'putchar'
other() : 'strlen'

a() in 0 to 9: return when label() is
    'putchar' then 1
    'labs' then 2
    else 3

b() in 0 to 9: return when other() is
    'putchar' then 1
    'labs' then 2
    else 3
";
    let module = check(source).expect("checks");
    assert_eq!(returned_number(&module, "a"), Some(1));
    // Neither text matches: the `else` arm.
    assert_eq!(returned_number(&module, "b"), Some(3));
}

/// Every arm is checked, not only the one a particular text selects.
#[test]
fn a_bad_arm_is_refused_whichever_text_is_chosen() {
    let source = "\
label() : 'putchar'

a() in 0 to 2: return when label() is
    'putchar' then 1
    'labs' then 9
    else 0
";
    assert!(check(source).is_err());
}

#[test]
fn text_is_an_open_set_so_a_case_over_it_needs_else() {
    let source = "label() : 'x'\n\na() in 0 to 9: return when label() is\n    'x' then 1\n    'y' then 2\n";
    assert!(check(source).is_err());
}

#[test]
fn a_repeated_text_is_a_duplicate() {
    let source = "label() : 'x'\n\na() in 0 to 9: return when label() is\n    'x' then 1\n    'x' then 2\n    else 3\n";
    assert_eq!(check(source).unwrap_err(), SourceCheckError::Duplicate);
}

#[test]
fn else_must_be_the_last_arm() {
    let source = "label() : 'x'\n\na() in 0 to 9: return when label() is\n    else 3\n    'x' then 1\n";
    // An `else` that opens the arms is read as continuing the line above, so it is refused
    // while parsing; either way the program is not accepted.
    assert!(
        parser::parse_strict(source).is_err()
            || check_module(
                "case.text",
                &parser::parse_strict(source).unwrap(),
                &[],
                Limits::default()
            )
            .is_err()
    );
}

/// A text arm over a number is a type error, not a silently ignored arm.
#[test]
fn a_text_arm_does_not_match_a_number() {
    let source = "a(n in 0 to 9) in 0 to 9: return when n is\n    'x' then 1\n    else 2\n";
    assert!(check(source).is_err());
}

#[test]
fn the_parameter_form_is_not_a_text_case() {
    // `pick(name)` takes text as an unannotated parameter only inside an instruction; here it is
    // not text at all, so the text case does not apply.
    assert!(check(PICK).is_err());
}
