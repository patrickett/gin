//! Records in recorded bodies: construction, destructuring, locals, and the compile-time
//! evaluation rule applied to them. A record is data to the evaluator, like an integer.
use i256::I256;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceBodyKind, CheckedSourceCallee, Evaluated, Limits,
    SourceCheckError, check_file, evaluate_value,
};

const REGISTER: &str = "Register has contents in 0 to 255\n";

fn check(text: &str) -> Result<typecheck::CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict record fixture");
    check_file(&file, Limits::default())
}

fn bodies(text: &str) -> (Vec<String>, Vec<Option<CheckedSourceBody>>) {
    let checked = check(&format!("{REGISTER}{text}")).expect("fixture checks");
    (
        checked.functions.iter().map(|f| f.name.clone()).collect(),
        checked.bodies,
    )
}

fn body(text: &str, name: &str) -> Option<CheckedSourceBody> {
    let (names, bodies) = bodies(text);
    bodies[names.iter().position(|n| n == name).unwrap()].clone()
}

/// The node the single `return` of the body yields.
fn returned(body: &CheckedSourceBody) -> &CheckedSourceBodyKind {
    let CheckedSourceBodyKind::Block { result, .. } = body.expressions[body.root as usize].kind
    else {
        panic!("root is a block");
    };
    let CheckedSourceBodyKind::Return { value } = body.expressions[result as usize].kind else {
        panic!("block ends in a return");
    };
    &body.expressions[value as usize].kind
}

struct NoBodies;

impl BodyResolver for NoBodies {
    fn body(&self, _: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        None
    }
}

fn integer(value: i64) -> Evaluated {
    Evaluated::Integer(I256::from(value))
}

#[test]
fn destructuring_a_known_construction_is_its_field() {
    let body = body(
        "make() in 0 to 255:\n    Register(contents) := Register(contents: 7)\n    return contents\n",
        "make",
    )
    .expect("recorded");
    assert_eq!(
        returned(&body),
        &CheckedSourceBodyKind::Integer(I256::from(7))
    );
}

#[test]
fn destructuring_an_unknown_record_reads_a_field() {
    let body = body(
        "read(register Register) in 0 to 255:\n    Register(contents) := register\n    return contents\n",
        "read",
    )
    .expect("recorded");
    assert!(matches!(
        returned(&body),
        CheckedSourceBodyKind::Field { index: 0, .. }
    ));
}

#[test]
fn a_record_body_is_evaluated_on_a_record_argument() {
    let body = body(
        "read(register Register) in 0 to 255:\n    Register(contents) := register\n    return contents\n",
        "read",
    )
    .unwrap();
    let evaluate = |argument| evaluate_value(&body, &[argument], &NoBodies, &mut 1_000);
    assert_eq!(
        evaluate(Evaluated::Record {
            schema: 0,
            fields: vec![integer(42)]
        }),
        Some(integer(42))
    );
    assert_eq!(evaluate(integer(42)), None, "an integer is not a record");
}

#[test]
fn a_known_record_argument_folds_the_call() {
    let body = body(
        "read(register Register) in 0 to 255:\n    Register(contents) := register\n    return contents\n\
         nine() in 0 to 255: return read(Register(contents: 9))\n",
        "nine",
    )
    .unwrap();
    assert_eq!(
        returned(&body),
        &CheckedSourceBodyKind::Integer(I256::from(9))
    );
}

#[test]
fn a_call_returning_a_known_record_folds_to_a_construction() {
    let body = body(
        "make() Register: return Register(contents: 5)\nagain() Register: return make()\n",
        "again",
    )
    .unwrap();
    let CheckedSourceBodyKind::Construct { fields, .. } = returned(&body) else {
        panic!("a known record is a construction, not a call");
    };
    assert_eq!(
        body.expressions[fields[0] as usize].kind,
        CheckedSourceBodyKind::Integer(I256::from(5))
    );
}

#[test]
fn a_record_of_unknown_fields_is_built_not_folded() {
    let body = body(
        "copy(register Register) Register:\n    Register(contents) := register\n    return Register(contents: contents)\n",
        "copy",
    )
    .unwrap();
    assert!(matches!(
        returned(&body),
        CheckedSourceBodyKind::Construct { .. }
    ));
}

#[test]
fn locals_name_nodes_and_shadow_parameters() {
    let body = body(
        "pick(left in 0 to 9) in 0 to 18:\n    right: 4\n    return left + right\n",
        "pick",
    )
    .unwrap();
    let CheckedSourceBodyKind::Binary { right, .. } = returned(&body) else {
        panic!("a sum of a parameter and a local");
    };
    assert_eq!(
        body.expressions[*right as usize].kind,
        CheckedSourceBodyKind::Integer(I256::from(4))
    );
}

#[test]
fn linearity_is_still_the_checkers_not_the_recorders() {
    for (text, error) in [
        (
            "twice(register Register) in 0 to 510:\n    Register(a) := register\n    Register(b) := register\n    return a + b\n",
            SourceCheckError::UseAfterMove,
        ),
        (
            "drop(register Register): return\n",
            SourceCheckError::Unconsumed,
        ),
    ] {
        assert_eq!(check(&format!("{REGISTER}{text}")).err(), Some(error));
    }
}

#[test]
fn what_is_not_recorded_stays_unrecorded() {
    // A generic function is recorded as a template (see `template_bodies.rs`); what the recorder
    // still cannot represent, here a local annotated with a type parameter, stays unrecorded.
    let checked = check(
        "Box(element) has value element\nunbox(element Type, box Box(element)) element:\n    Box(value) := box\n    return value\n",
    )
    .unwrap();
    assert!(checked.bodies[0].is_some());
}
