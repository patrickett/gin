//! Sums in recorded bodies: members carry their schema, `when` records every arm for an
//! unknown member, and a known member is its arm and nothing else.
use i256::I256;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceBodyKind, CheckedSourceCallee, CheckedSourceFile,
    Evaluated, Limits, check_file, evaluate_value,
};

const REGISTERS: &str = "Register has contents in 0 to 255
ArmRegister is X0(Register) or X1(Register)

take(register Register) in 0 to 255:
    Register(contents) := register
    return contents

contents(arm ArmRegister) in 0 to 256:
    return when arm is
        X0(register) then take(register)
        X1(register) then take(register) + 1
";

fn check(extra: &str) -> CheckedSourceFile {
    let text = format!("{REGISTERS}{extra}");
    let file = parser::parse_strict(&text).expect("strict sum fixture");
    check_file(&file, Limits::default()).expect("fixture checks")
}

fn body(checked: &CheckedSourceFile, name: &str) -> CheckedSourceBody {
    let index = checked
        .functions
        .iter()
        .position(|f| f.name == name)
        .unwrap();
    checked.bodies[index].clone().expect("recorded")
}

fn schema(checked: &CheckedSourceFile, name: &str) -> u32 {
    checked
        .aggregates
        .iter()
        .position(|a| a.name == name)
        .unwrap() as u32
}

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

struct Table<'a>(&'a CheckedSourceFile);

impl BodyResolver for Table<'_> {
    fn body(&self, callee: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        let CheckedSourceCallee::Local { function } = callee else {
            return None;
        };
        self.0.bodies.get(function as usize)?.as_ref()
    }
}

fn member(checked: &CheckedSourceFile, name: &str, contents: i64) -> Evaluated {
    Evaluated::Record {
        schema: schema(checked, name),
        fields: vec![Evaluated::Record {
            schema: schema(checked, "Register"),
            fields: vec![Evaluated::Integer(I256::from(contents))],
        }],
    }
}

#[test]
fn an_unknown_member_records_every_arm() {
    let checked = check("");
    let body = body(&checked, "contents");
    let CheckedSourceBodyKind::Match { arms, .. } = returned(&body) else {
        panic!("a parameter of a sum is matched at run time");
    };
    let schemas: Vec<u32> = arms.iter().map(|arm| arm.schema).collect();
    assert_eq!(schemas, [schema(&checked, "ArmRegister.X0"), schema(&checked, "ArmRegister.X1")]);
}

#[test]
fn the_evaluator_takes_the_arm_of_the_member_it_holds() {
    let checked = check("");
    let body = body(&checked, "contents");
    let table = Table(&checked);
    let evaluate = |argument| evaluate_value(&body, &[argument], &table, &mut 1_000);
    assert_eq!(
        evaluate(member(&checked, "ArmRegister.X0", 20)),
        Some(Evaluated::Integer(I256::from(20)))
    );
    assert_eq!(
        evaluate(member(&checked, "ArmRegister.X1", 20)),
        Some(Evaluated::Integer(I256::from(21)))
    );
}

#[test]
fn a_known_member_is_its_arm_and_nothing_else() {
    let checked = check(
        "start() ArmRegister: return X1(Register(contents: 9))
pick() in 0 to 256:
    return when start() is
        X0(register) then take(register)
        X1(register) then take(register) + 1
",
    );
    let body = body(&checked, "pick");
    assert_eq!(
        returned(&body),
        &CheckedSourceBodyKind::Integer(I256::from(10))
    );
}

#[test]
fn a_call_on_a_known_member_folds() {
    let checked =
        check("seven() in 0 to 256: return contents(X0(Register(contents: 7)))\n");
    let body = body(&checked, "seven");
    assert_eq!(
        returned(&body),
        &CheckedSourceBodyKind::Integer(I256::from(7))
    );
}

#[test]
fn a_call_returning_a_known_member_folds_to_that_member() {
    let checked = check(
        "bump(arm ArmRegister) ArmRegister:
    return when arm is
        X0(register) then X1(register)
        X1(register) then X0(register)
start() ArmRegister: return X0(Register(contents: 3))
flipped() ArmRegister: return bump(start())
",
    );
    let body = body(&checked, "flipped");
    let CheckedSourceBodyKind::Construct { schema: member, .. } = returned(&body) else {
        panic!("a known member is a construction");
    };
    assert_eq!(*member, schema(&checked, "ArmRegister.X1"));
}

#[test]
fn a_member_without_fields_is_a_construction() {
    let file = parser::parse_strict(
        "Cell has value in 0 to 255\nSlot is Empty or Full(Cell)\nnothing() Slot: return Empty\n",
    )
    .unwrap();
    let checked = check_file(&file, Limits::default()).unwrap();
    let body = body(&checked, "nothing");
    assert!(matches!(
        returned(&body),
        CheckedSourceBodyKind::Construct { fields, .. } if fields.is_empty()
    ));
}
