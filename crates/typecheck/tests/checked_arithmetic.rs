//! `+`, `-` and `*` in value position, and the compile-time evaluation rule:
//! when every input of an operation is known, the body holds its value, not the operation.
use i256::I256;
use typecheck::{
    BodyResolver, CheckedArithmetic, CheckedSourceBody, CheckedSourceBodyKind, CheckedSourceCallee,
    Limits, SourceCheckError, SourceType, check_file, evaluate_body,
};

fn check(text: &str) -> Result<typecheck::CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict arithmetic fixture");
    check_file(&file, Limits::default())
}

fn body(text: &str, function: usize) -> CheckedSourceBody {
    check(text).expect("fixture checks").bodies[function]
        .clone()
        .expect("body was recorded")
}

/// The expression the single `return` of `body` yields.
fn returned(body: &CheckedSourceBody) -> &typecheck::CheckedSourceBodyExpr {
    let CheckedSourceBodyKind::Block { result, .. } = body.expressions[body.root as usize].kind
    else {
        panic!("root is a block");
    };
    let CheckedSourceBodyKind::Return { value } = body.expressions[result as usize].kind else {
        panic!("block ends in a return");
    };
    &body.expressions[value as usize]
}

fn domain(lower: i64, upper: i64) -> SourceType {
    SourceType::Integer {
        lower: I256::from(lower),
        upper: I256::from(upper),
    }
}

#[test]
fn addition_has_the_exact_sum_domain_and_is_recorded() {
    let body = body(
        "sum(a in 0 to 100, b in 0 to 100) in 0 to 200: return a + b\n",
        0,
    );
    let sum = returned(&body);
    assert_eq!(sum.ty, domain(0, 200));
    let CheckedSourceBodyKind::Binary {
        operator,
        left,
        right,
    } = sum.kind
    else {
        panic!("a sum of parameters stays an operation");
    };
    assert_eq!(operator, CheckedArithmetic::Add);
    assert_eq!(
        body.expressions[left as usize].kind,
        CheckedSourceBodyKind::Parameter { index: 0 }
    );
    assert_eq!(
        body.expressions[right as usize].kind,
        CheckedSourceBodyKind::Parameter { index: 1 }
    );
}

#[test]
fn subtraction_and_multiplication_domains_are_exact() {
    let difference = body(
        "diff(a in 0 to 100, b in 0 to 100) in -100 to 100: return a - b\n",
        0,
    );
    assert_eq!(returned(&difference).ty, domain(-100, 100));
    let scaled = body("scaled(a in 0 to 10) in 0 to 30: return a * 3\n", 0);
    assert_eq!(returned(&scaled).ty, domain(0, 30));
}

#[test]
fn a_result_outside_the_declared_domain_is_rejected() {
    for text in [
        "sum(a in 0 to 100, b in 0 to 100) in 0 to 199: return a + b\n",
        "diff(a in 0 to 100, b in 0 to 100) in 0 to 100: return a - b\n",
        "scaled(a in 0 to 10) in 0 to 29: return a * 3\n",
    ] {
        assert!(check(text).is_err(), "{text:?} must not check");
    }
}

#[test]
fn operations_on_known_values_are_evaluated_when_recorded() {
    let folded = body("answer() in 0 to 16: return 2 + 3 * 4\n", 0);
    assert_eq!(
        returned(&folded).kind,
        CheckedSourceBodyKind::Integer(I256::from(14))
    );
    // The operands left no nodes: the literal, the return and the block remain.
    assert_eq!(folded.expressions.len(), 3);
    assert!(
        folded
            .expressions
            .iter()
            .all(|node| !matches!(node.kind, CheckedSourceBodyKind::Binary { .. }))
    );
}

#[test]
fn a_known_operand_beside_an_unknown_one_is_not_folded() {
    let body = body("next(x in 0 to 15) in 0 to 16: return x + 1\n", 0);
    assert!(matches!(
        returned(&body).kind,
        CheckedSourceBodyKind::Binary { .. }
    ));
    assert_eq!(returned(&body).ty, domain(1, 16));
}

#[test]
fn calls_with_known_arguments_are_their_value() {
    let source = "five() in 0 to 16: return 5\n\
                  again() in 0 to 16: return five() + 1\n";
    assert_eq!(
        returned(&body(source, 1)).kind,
        CheckedSourceBodyKind::Integer(I256::from(6))
    );
    let source = "double(x in 0 to 8) in 0 to 16: return x + x\n\
                  nine() in 0 to 17: return double(4) + 1\n";
    // The checker proves only `1 to 17` for `double(4) + 1`, because callee result
    // domains are not specialised by call arguments. Evaluation knows the value: 9.
    assert_eq!(
        returned(&body(source, 1)).kind,
        CheckedSourceBodyKind::Integer(I256::from(9))
    );
}

#[test]
fn the_evaluator_runs_a_recorded_body_on_constants() {
    struct NoBodies;
    impl BodyResolver for NoBodies {
        fn body(&self, _: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
            None
        }
    }
    let body = body(
        "sum(a in 0 to 100, b in 0 to 100) in 0 to 200: return a + b\n",
        0,
    );
    let mut fuel = 100;
    let args = [I256::from(30), I256::from(12)];
    assert_eq!(
        evaluate_body(&body, &args, &NoBodies, &mut fuel),
        Some(I256::from(42))
    );
    // An argument outside its parameter's domain is not evaluated.
    let args = [I256::from(101), I256::from(1)];
    assert_eq!(evaluate_body(&body, &args, &NoBodies, &mut 100), None);
    // Too little fuel stays unknown instead of guessing.
    let args = [I256::from(1), I256::from(1)];
    assert_eq!(evaluate_body(&body, &args, &NoBodies, &mut 2), None);
}
