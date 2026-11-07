//! `#infix('op')` makes `a op b` call a method of the left operand's type.
use i256::I256;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceCallee, CheckedSourceFile, Evaluated,
    InfixOperator, Limits, SourceCheckError, check_file, evaluate_value,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict infix fixture");
    check_file(&file, Limits::default())
}

const MONEY: &str = "\
#unrestricted
Money has
  cents in 0 to 99
  #infix('+')
  add(self, rhs Self) in 0 to 198:
    Money(a) := self
    Money(b) := rhs
    return a + b
";

fn with(tail: &str) -> String {
    format!("{MONEY}{tail}")
}

#[test]
fn an_operator_on_a_nominal_type_calls_its_infix_method() {
    let checked = check(&with("total(a Money, b Money) in 0 to 198: return a + b\n")).unwrap();
    let add = checked
        .functions
        .iter()
        .find(|f| f.name == "Money.add")
        .unwrap();
    assert_eq!(add.infix, Some(InfixOperator::Add));
    let total = checked
        .functions
        .iter()
        .find(|f| f.name == "total")
        .unwrap();
    assert_eq!(total.infix, None);
}

#[test]
fn self_names_the_receiver_type_in_a_methods_signature() {
    let source = "\
#unrestricted
Money has
  cents in 0 to 99
  same(self, other Self) Self: self
";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn a_nominal_type_without_the_operator_has_no_arithmetic() {
    let source = with("diff(a Money, b Money) in 0 to 198: return a - b\n");
    assert_eq!(check(&source), Err(SourceCheckError::Type));
    let plain =
        "Money has\n  cents in 0 to 99\ntotal(a Money, b Money) in 0 to 198: return a + b\n";
    assert_eq!(check(plain), Err(SourceCheckError::Type));
}

#[test]
fn dispatch_follows_the_left_operand_only() {
    let source = with("total(a Money) in 0 to 198: return 5 + a\n");
    assert_eq!(check(&source), Err(SourceCheckError::Type));
}

#[test]
fn the_right_operand_converts_to_the_declared_rhs_type() {
    let source = "\
#unrestricted
Money has
  cents in 0 to 99
  #infix('*')
  scale(self, by in 0 to 3) in 0 to 297:
    Money(a) := self
    return a * by
double(a Money) in 0 to 297: return a * 2
";
    assert!(check(source).is_ok(), "{:?}", check(source));
    let too_big = source.replace("a * 2", "a * 9");
    assert!(check(&too_big).is_err());
}

#[test]
fn linear_operands_are_consumed_by_the_operator() {
    let linear = MONEY.replace("#unrestricted\n", "");
    let once = format!("{linear}total(a Money, b Money) in 0 to 198: return a + b\n");
    assert!(check(&once).is_ok(), "{:?}", check(&once));
    let twice = format!("{linear}total(a Money) in 0 to 198: return a + a\n");
    assert_eq!(check(&twice), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn an_infix_method_needs_a_supported_operator_and_a_receiver_with_one_operand() {
    let source = |attribute: &str, method: &str| {
        format!("#unrestricted\nMoney has\n  cents in 0 to 99\n  {attribute}\n  {method}\n")
    };
    let operand = "add(self, rhs Self) in 0 to 1: 0";
    assert_eq!(
        check(&source("#infix('**')", operand)),
        Err(SourceCheckError::InfixOperator)
    );
    assert_eq!(
        check(&source("#infix('+')", "add(self) in 0 to 1: 0")),
        Err(SourceCheckError::InfixOperator)
    );
    assert_eq!(
        check(&source(
            "#infix('+')",
            "add(self, a Self, b Self) in 0 to 1: 0"
        )),
        Err(SourceCheckError::InfixOperator)
    );
    assert_eq!(
        check(&source(
            "#infix('+')",
            "add(rhs Self, other Self) in 0 to 1: 0"
        )),
        Err(SourceCheckError::MethodReceiver)
    );
}

#[test]
fn one_operator_per_type_and_one_operator_per_method() {
    let two_methods = "\
#unrestricted
Money has
  cents in 0 to 99
  #infix('+')
  add(self, rhs Self) in 0 to 1: 0
  #infix('+')
  plus(self, rhs Self) in 0 to 1: 0
";
    assert_eq!(check(two_methods), Err(SourceCheckError::Duplicate));
    let two_operators = "\
#unrestricted
Money has
  cents in 0 to 99
  #infix('+'), #infix('-')
  add(self, rhs Self) in 0 to 1: 0
";
    assert_eq!(check(two_operators), Err(SourceCheckError::Duplicate));
}

struct Bodies<'a>(&'a [Option<CheckedSourceBody>]);

impl BodyResolver for Bodies<'_> {
    fn body(&self, callee: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        match callee {
            CheckedSourceCallee::Local { function } => self.0[function as usize].as_ref(),
            CheckedSourceCallee::Dependency { .. } => None,
        }
    }
}

fn money(cents: i64) -> Evaluated {
    Evaluated::Record {
        schema: 0,
        fields: vec![Evaluated::Integer(I256::from(cents))],
    }
}

#[test]
fn an_infix_use_is_recorded_as_a_call_and_evaluates_through_the_method() {
    let checked = check(&with("total(a Money, b Money) in 0 to 198: return a + b\n")).unwrap();
    let index = checked
        .functions
        .iter()
        .position(|f| f.name == "total")
        .unwrap();
    let body = checked.bodies[index].as_ref().expect("recorded");
    let result = evaluate_value(
        body,
        &[money(40), money(35)],
        &Bodies(&checked.bodies),
        &mut 1_000,
    );
    assert_eq!(result, Some(Evaluated::Integer(I256::from(75))));
}
