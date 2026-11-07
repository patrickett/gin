//! The last expression of a function's body is its result; `return` is for leaving early.
use typecheck::{CheckedSourceBodyKind, CheckedSourceFile, Limits, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, typecheck::SourceCheckError> {
    let file = parser::parse_strict(text).expect("parses");
    check_file(&file, Limits::default())
}

fn returned_kind(checked: &CheckedSourceFile, name: &str) -> CheckedSourceBodyKind {
    let index = checked.functions.iter().position(|f| f.name == name).unwrap();
    let body = checked.bodies[index].clone().expect("recorded");
    let CheckedSourceBodyKind::Block { result, .. } = body.expressions[body.root as usize].kind
    else {
        panic!("block body");
    };
    let CheckedSourceBodyKind::Return { value } = body.expressions[result as usize].kind else {
        panic!("the tail is recorded as the result");
    };
    body.expressions[value as usize].kind.clone()
}

#[test]
fn a_local_then_its_name_is_the_result() {
    let checked = check("double(n in 0 to 100) in 0 to 200:\n    twice: n + n\n    twice\n").unwrap();
    assert!(matches!(
        returned_kind(&checked, "double"),
        CheckedSourceBodyKind::Binary { .. }
    ));
}

#[test]
fn a_guard_that_returns_precedes_the_tail() {
    check("pick(n in 0 to 100) in 0 to 200:\n    when n < 50 then return n\n    n + 100\n").unwrap();
}

#[test]
fn a_tail_outside_the_declared_result_is_rejected() {
    // 100 + 100 does not fit in 0 to 150.
    assert!(check("over(n in 0 to 100) in 0 to 150:\n    n + 100\n").is_err());
}

#[test]
fn a_unit_function_may_end_in_a_unit_expression() {
    check("noop():\n    return\nwrapper():\n    noop()\n").unwrap();
}
