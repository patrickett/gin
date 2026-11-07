//! Executable-body recording through the public source interface.
use i256::I256;
use typecheck::{CheckedSourceBodyKind, Limits, SourceType, check_file};

fn check(text: &str) -> typecheck::CheckedSourceFile {
    let file = parser::parse_strict(text).expect("strict checked-body fixture");
    check_file(&file, Limits::default()).expect("checked-body fixture type checks")
}

fn body(text: &str) -> typecheck::CheckedSourceBody {
    let checked = check(text);
    assert_eq!(checked.functions.len(), 1, "single function in {text:?}");
    checked.bodies[0]
        .as_ref()
        .cloned()
        .expect("body was recorded")
}

#[test]
fn parameter_echo_body_is_recorded() {
    let body =
        body("echo(value in 0 to 16) out in 0 to 16 where out >= value: return value\n");
    let root = &body.expressions[body.root as usize];
    let CheckedSourceBodyKind::Block { statements, result } = &root.kind else {
        panic!("echo root is a block");
    };
    assert_eq!(statements.len(), 1);
    assert_eq!(*result, statements[0]);
    let returned = &body.expressions[*result as usize];
    let CheckedSourceBodyKind::Return { value } = returned.kind else {
        panic!("echo result is a return");
    };
    let parameter = &body.expressions[value as usize];
    assert_eq!(
        parameter.kind,
        CheckedSourceBodyKind::Parameter { index: 0 }
    );
    assert_eq!(
        parameter.ty,
        SourceType::Integer {
            lower: I256::ZERO,
            upper: I256::from(16),
        }
    );
}

#[test]
fn constant_body_is_recorded() {
    let body = body("answer() in 0 to 5: return 3\n");
    let root = &body.expressions[body.root as usize];
    let CheckedSourceBodyKind::Block { result, .. } = &root.kind else {
        panic!("constant root is a block");
    };
    let returned = &body.expressions[*result as usize];
    let CheckedSourceBodyKind::Return { value } = returned.kind else {
        panic!("constant result is a return");
    };
    let literal = &body.expressions[value as usize];
    assert_eq!(literal.kind, CheckedSourceBodyKind::Integer(I256::from(3)));
    assert_eq!(
        literal.ty,
        SourceType::Integer {
            lower: I256::from(3),
            upper: I256::from(3),
        }
    );
}

#[test]
fn bare_return_body_is_recorded_as_unit() {
    let body = body("answer(): return\n");
    let root = &body.expressions[body.root as usize];
    let CheckedSourceBodyKind::Block { result, .. } = &root.kind else {
        panic!("bare-return root is a block");
    };
    let returned = &body.expressions[*result as usize];
    let CheckedSourceBodyKind::Return { value } = returned.kind else {
        panic!("bare-return result is a return");
    };
    assert_eq!(
        body.expressions[value as usize].kind,
        CheckedSourceBodyKind::Unit
    );
}

#[test]
fn an_integer_range_local_is_recorded_as_its_value() {
    let checked = check("answer() in 0 to 5:\n  x in 0 to 5: 3\n  return x\n");
    assert_eq!(checked.functions.len(), 1);
    assert!(checked.bodies[0].is_some(), "a range annotation needs no conversion");
}
