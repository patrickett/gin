//! Any binary operator a nominal type defines with `#infix('op')` dispatches to its method,
//! which returns whatever type it declares: a library, not the compiler, gives it meaning.
use i256::I256;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceCallee, CheckedSourceFile, Evaluated,
    InfixOperator, Limits, SourceCheckError, check_file, evaluate_value,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict operator fixture");
    check_file(&file, Limits::default())
}

const TRUTH: &str = "\
#unrestricted
Truth is True or False
";

const COUNT: &str = "\
#unrestricted
Count is in 0 to 9
Count has
  #infix('=')
  equals(self ref, rhs ref Self) Truth: True
  #infix('<')
  less(self ref, rhs ref Self) Truth: False
";

fn program(tail: &str) -> String {
    format!("{TRUTH}{COUNT}{tail}")
}

#[test]
fn comparison_operators_are_defined_by_the_type_and_return_what_it_declares() {
    let source = program(
        "same(a Count, b Count) Truth: return a = b\nbelow(a Count, b Count) Truth: return a < b\n",
    );
    let checked = check(&source).unwrap();
    let result = |name: &str| {
        checked
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .result
    };
    assert_eq!(result("same"), result("Count.equals"));
    assert_eq!(
        checked
            .functions
            .iter()
            .find(|f| f.name == "Count.less")
            .unwrap()
            .infix,
        Some(InfixOperator::Less)
    );
}

#[test]
fn a_comparison_result_is_an_ordinary_value_a_match_consumes() {
    let source = program(
        "pick(a Count, b Count) in 0 to 1:\n  return when a < b is\n    True then 1\n    False then 0\n",
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn every_binary_operator_can_be_defined_by_a_library() {
    for operator in InfixOperator::ALL {
        let symbol = operator.symbol();
        let source = format!(
            "{TRUTH}#unrestricted\nCell is in 0 to 9\nCell has\n  #infix('{symbol}')\n  op(self ref, rhs ref Self) Truth: True\nuse_it(a Cell, b Cell) Truth: return a {symbol} b\n"
        );
        let checked = check(&source);
        assert!(checked.is_ok(), "{symbol}: {checked:?}");
    }
}

#[test]
fn an_operator_a_type_does_not_define_is_a_type_error() {
    let source = program("use_it(a Count, b Count) Truth: return a <= b\n");
    assert_eq!(check(&source), Err(SourceCheckError::Type));
}

#[test]
fn comparing_lends_the_operands_and_never_consumes_a_linear_value() {
    let source = format!(
        "{TRUTH}Money is in 0 to 9\nMoney has\n  close(self): return\n  #infix('<')\n  less(self ref, rhs ref Self) Truth: False\ncheck(a Money, b Money) Truth:\n  r: a < b\n  a.close\n  b.close\n  return r\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
    let unfinished = source.replace("  a.close\n", "");
    assert_eq!(check(&unfinished), Err(SourceCheckError::Unconsumed));
}

#[test]
fn one_value_may_appear_on_both_sides_of_a_shared_comparison() {
    let source = program("reflexive(a Count) Truth: return a = a\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn integers_keep_their_native_operators() {
    let source = "f(a in 0 to 5, b in 0 to 5) in 0 to 10: return a + b\n";
    assert!(check(source).is_ok());
    assert!(check("g(a in 0 to 5, b in 0 to 5) in 0 to 5: return a < b\n").is_err());
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

#[test]
fn a_comparison_records_and_evaluates_to_the_value_the_method_returns() {
    let source = program("below(a ref Count, b ref Count) Truth: return a < b\n");
    let checked = check(&source).unwrap();
    let index = checked
        .functions
        .iter()
        .position(|f| f.name == "below")
        .unwrap();
    let body = checked.bodies[index].as_ref().expect("recorded");
    let count = |n: i64| Evaluated::Record {
        schema: 3,
        fields: vec![Evaluated::Integer(I256::from(n))],
    };
    let result = evaluate_value(
        body,
        &[count(1), count(2)],
        &Bodies(&checked.bodies),
        &mut 1_000,
    );
    // `less` returns `False`, the second record declared.
    assert_eq!(
        result,
        Some(Evaluated::Record {
            schema: 3,
            fields: vec![]
        })
    );
}
