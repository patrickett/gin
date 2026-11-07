//! Methods: `has` members whose first parameter is `self` are functions `Type.name`.
use i256::I256;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceCallee, CheckedSourceFile, Evaluated, Limits,
    SourceCheckError, SourceType, check_file, evaluate_value,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict method fixture");
    check_file(&file, Limits::default())
}

const POINT: &str = "\
Point has
  x in 0 to 9
  y in 0 to 9
  sum(self) in 0 to 18:
    Point(x, y) := self
    return x + y
";

#[test]
fn a_method_is_a_qualified_function_and_not_a_field() {
    let checked = check(POINT).unwrap();
    assert_eq!(checked.aggregates[0].fields.len(), 2);
    assert_eq!(checked.functions.len(), 1);
    assert_eq!(checked.functions[0].name, "Point.sum");
    assert!(matches!(
        checked.functions[0].parameters[..],
        [SourceType::Aggregate { .. }]
    ));
}

#[test]
fn methods_and_fields_may_interleave() {
    let source = "#unrestricted\nPoint has\n  x in 0 to 9\n  keep(self): return\n  y in 0 to 9\n";
    let checked = check(source).unwrap();
    let fields: Vec<_> = checked.aggregates[0]
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(fields, ["x", "y"]);
    assert_eq!(checked.functions[0].name, "Point.keep");
}

#[test]
fn bare_self_is_a_by_value_receiver_that_must_be_consumed() {
    let leaky = "Point has\n  x in 0 to 9\n  keep(self): return\n";
    assert_eq!(check(leaky), Err(SourceCheckError::Unconsumed));
    let unrestricted = format!("#unrestricted\n{leaky}");
    assert!(check(&unrestricted).is_ok());
}

#[test]
fn a_receiver_is_owned_unless_it_borrows_or_ends() {
    for word in ["ref", "mut"] {
        let source = format!("Point has\n  x in 0 to 9\n  m(self {word}): return\n");
        assert!(check(&source).is_ok(), "{word}: {:?}", check(&source));
    }
    // `end` is an owner that must also end the value, so a body that drops it is refused.
    let ends = "Point has\n  x in 0 to 9\n  m(self end): return\n";
    assert_eq!(check(ends), Err(SourceCheckError::Unconsumed));
    // The legacy `own` transfer marker has no meaning here.
    let legacy = "Point has\n  x in 0 to 9\n  m(self own): return\n";
    assert_eq!(check(legacy), Err(SourceCheckError::Unsupported));
}

#[test]
fn every_parameterised_member_is_a_method_but_self_must_come_first() {
    for header in ["helper(a in 0 to 1): return", "helper(): return"] {
        let source = format!("Point has\n  x in 0 to 9\n  {header}\n");
        let checked = check(&source).expect(header);
        assert_eq!(checked.functions[0].name, "Point.helper");
        assert_eq!(checked.aggregates[0].fields.len(), 1);
    }
    let late = "Point has\n  x in 0 to 9\n  helper(a in 0 to 1, self): return\n";
    assert_eq!(check(late), Err(SourceCheckError::MethodReceiver));
}

#[test]
fn self_in_a_body_requires_a_self_parameter() {
    let source =
        "Point has\n  x in 0 to 9\n  helper(a in 0 to 1):\n    Point(x) := self\n    return\n";
    assert_eq!(check(source), Err(SourceCheckError::UnknownName));
}

#[test]
fn a_field_and_a_method_cannot_share_a_name() {
    let source = "#unrestricted\nPoint has\n  x in 0 to 9\n  x(self): return\n";
    assert_eq!(check(source), Err(SourceCheckError::Duplicate));
}

#[test]
fn duplicate_method_names_collide_but_other_types_may_reuse_them() {
    let twice = "#unrestricted\nPoint has\n  x in 0 to 9\n  m(self): return\n  m(self): return\n";
    assert_eq!(check(twice), Err(SourceCheckError::Duplicate));
    let two_types = "#unrestricted\nA has\n  x in 0 to 9\n  m(self): return\n#unrestricted\nB has\n  y in 0 to 9\n  m(self): return\n";
    let checked = check(two_types).unwrap();
    let names: Vec<_> = checked.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["A.m", "B.m"]);
}

#[test]
fn a_method_does_not_collide_with_a_free_function_of_the_same_name() {
    let source =
        "#unrestricted\nPoint has\n  x in 0 to 9\n  m(self): return\nm(v in 0 to 1): return\n";
    assert!(check(source).is_ok());
}

#[test]
fn a_generic_record_has_static_methods_over_its_own_parameters_or_ones_of_their_own() {
    // A method with no receiver uses the type's parameters by name ...
    let source = "Box(element) has\n  item element\n  m(): return\n";
    assert!(check(source).is_ok());
    // ... or writes type parameters of its own (`Allocation.reclaim_storage(x, ...)`).
    let source = "Box(element) has\n  item element\n  m(other): return\n";
    assert!(check(source).is_ok());
}

const SUM: &str = "\
Point has
  x in 0 to 9
  y in 0 to 9
  sum(self) in 0 to 18:
    Point(x, y) := self
    return x + y
";

fn with(tail: &str) -> String {
    format!("{SUM}{tail}")
}

#[test]
fn a_method_call_passes_the_receiver_first_and_has_the_declared_result() {
    let checked = check(&with("total(p Point) in 0 to 18: return p.sum()\n")).unwrap();
    let total = checked
        .functions
        .iter()
        .find(|f| f.name == "total")
        .unwrap();
    assert_eq!(
        total.result,
        SourceType::Integer {
            lower: 0.into(),
            upper: 18.into()
        }
    );
}

#[test]
fn a_method_call_consumes_a_linear_receiver() {
    let reuse = with("twice(p Point) in 0 to 36:\n  a: p.sum()\n  b: p.sum()\n  return a + b\n");
    assert_eq!(check(&reuse), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn an_unrestricted_receiver_may_be_called_repeatedly() {
    let source = "\
#unrestricted
Point has
  x in 0 to 9
  y in 0 to 9
  sum(self) in 0 to 18:
    Point(x, y) := self
    return x + y
twice(p Point) in 0 to 36:
  a: p.sum()
  b: p.sum()
  return a + b
";
    assert!(check(source).is_ok());
}

#[test]
fn method_arguments_follow_the_receiver() {
    let source = "\
Point has
  x in 0 to 9
  scaled(self, k in 0 to 2) in 0 to 18:
    Point(x) := self
    return x * k
use_it(p Point) in 0 to 18: return p.scaled(2)
";
    assert!(check(source).is_ok());
    let wrong = source.replace("p.scaled(2)", "p.scaled(2, 2)");
    assert_eq!(check(&wrong), Err(SourceCheckError::Type));
    let out_of_range = source.replace("p.scaled(2)", "p.scaled(3)");
    assert!(check(&out_of_range).is_err());
}

#[test]
fn the_receiver_type_selects_between_same_named_methods() {
    let source = "\
A has
  x in 0 to 9
  tag(self) in 1 to 1:
    A(x) := self
    return 1
B has
  y in 0 to 9
  tag(self) in 2 to 2:
    B(y) := self
    return 2
from_a(a A) in 1 to 1: return a.tag()
from_b(b B) in 2 to 2: return b.tag()
";
    assert!(check(source).is_ok());
    let mixed = source.replace("from_a(a A) in 1 to 1", "from_a(a A) in 2 to 2");
    assert!(check(&mixed).is_err());
}

#[test]
fn unknown_methods_and_unsupported_receivers_are_rejected() {
    assert!(check(&with("f(p Point) in 0 to 18: return p.missing()\n")).is_err());
    let nested = with("g(p Point) in 0 to 18: return h(p).sum()\nh(p Point) Point: return p\n");
    assert!(check(&nested).is_err());
}

#[test]
fn a_method_with_only_self_may_be_called_without_parentheses() {
    let bare = with("total(p Point) in 0 to 18: return p.sum\n");
    let called = with("total(p Point) in 0 to 18: return p.sum()\n");
    let bare = check(&bare).unwrap();
    let called = check(&called).unwrap();
    assert_eq!(bare.functions, called.functions);
}

#[test]
fn a_bare_method_call_still_consumes_a_linear_receiver() {
    let reuse = with("twice(p Point) in 0 to 36:\n  a: p.sum\n  b: p.sum\n  return a + b\n");
    assert_eq!(check(&reuse), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn a_method_needing_arguments_cannot_drop_its_parentheses() {
    let source = "\
Point has
  x in 0 to 9
  scaled(self, k in 0 to 2) in 0 to 18:
    Point(x) := self
    return x * k
use_it(p Point) in 0 to 18: return p.scaled
";
    assert_eq!(check(source), Err(SourceCheckError::Type));
}

#[test]
fn a_field_read_is_not_mistaken_for_a_method_and_copies_an_unrestricted_field() {
    let source = "\
#unrestricted
Point has
  x in 0 to 9
  y in 0 to 9
  sum(self) in 0 to 18:
    Point(x, y) := self
    return x + y
first(p Point) in 0 to 9: return p.x
";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

const ORIGIN: &str = "\
Point has
  x in 0 to 9
  origin() in 0 to 0: return 0
  add(a in 0 to 5, b in 0 to 5) in 0 to 10: return a + b
";

#[test]
fn a_method_without_self_is_called_on_the_type_with_or_without_parentheses() {
    for call in ["Point.origin()", "Point.origin"] {
        let source = format!("{ORIGIN}go() in 0 to 0: return {call}\n");
        assert!(check(&source).is_ok(), "{call}");
    }
    let source = format!("{ORIGIN}go() in 0 to 10: return Point.add(1, 2)\n");
    assert!(check(&source).is_ok());
}

#[test]
fn type_path_calls_check_arity_and_domains() {
    let arity = format!("{ORIGIN}go() in 0 to 10: return Point.add(1)\n");
    assert_eq!(check(&arity), Err(SourceCheckError::Type));
    let range = format!("{ORIGIN}go() in 0 to 10: return Point.add(9, 1)\n");
    assert!(check(&range).is_err());
}

#[test]
fn a_method_with_self_may_also_be_called_through_the_type() {
    let source = with("go(p Point) in 0 to 18: return Point.sum(p)\n");
    assert!(check(&source).is_ok());
    let consumed = with("go(p Point) in 0 to 36:\n  a: Point.sum(p)\n  b: p.sum\n  return a + b\n");
    assert_eq!(check(&consumed), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn a_receiver_cannot_call_a_method_that_lacks_self() {
    let source =
        format!("{ORIGIN}go(p Point) in 0 to 0:\n  a: p.origin()\n  Point(x) := p\n  return a\n");
    assert_eq!(check(&source), Err(SourceCheckError::Type));
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

fn int(value: i64) -> Evaluated {
    Evaluated::Integer(I256::from(value))
}

fn point(x: i64, y: i64) -> Evaluated {
    Evaluated::Record {
        schema: 0,
        fields: vec![int(x), int(y)],
    }
}

fn run(checked: &CheckedSourceFile, function: &str, arguments: &[Evaluated]) -> Option<Evaluated> {
    let index = checked.functions.iter().position(|f| f.name == function)?;
    let body = checked.bodies[index].as_ref()?;
    evaluate_value(body, arguments, &Bodies(&checked.bodies), &mut 10_000)
}

#[test]
fn every_call_spelling_is_recorded_and_evaluates_through_the_method() {
    let source = with(concat!(
        "bare(p Point) in 0 to 18: return p.sum\n",
        "parens(p Point) in 0 to 18: return p.sum()\n",
        "path(p Point) in 0 to 18: return Point.sum(p)\n",
    ));
    let checked = check(&source).unwrap();
    for function in ["bare", "parens", "path"] {
        assert_eq!(
            run(&checked, function, &[point(3, 4)]),
            Some(int(7)),
            "{function}"
        );
    }
}

#[test]
fn associated_functions_and_extra_arguments_are_recorded_and_evaluated() {
    let source = format!("{ORIGIN}go(a in 0 to 5) in 0 to 10: return Point.add(a, 2)\n");
    let checked = check(&source).unwrap();
    assert_eq!(run(&checked, "go", &[int(4)]), Some(int(6)));
    let scaled = "\
Point has
  x in 0 to 9
  scaled(self, k in 0 to 2) in 0 to 18:
    Point(x) := self
    return x * k
go(p Point, k in 0 to 2) in 0 to 18: return p.scaled(k)
";
    let checked = check(scaled).unwrap();
    let one = Evaluated::Record {
        schema: 0,
        fields: vec![int(5)],
    };
    assert_eq!(run(&checked, "go", &[one, int(2)]), Some(int(10)));
}

#[test]
fn a_call_with_a_known_receiver_is_folded_when_recorded() {
    let source = with("go() in 0 to 18:\n  p: Point(x: 3, y: 4)\n  return p.sum\n");
    let checked = check(&source).unwrap();
    assert_eq!(run(&checked, "go", &[]), Some(int(7)));
}

#[test]
fn receivers_that_are_not_names_are_not_recorded() {
    let source = with("go(p Point) in 0 to 18: return id(p).sum\nid(p Point) Point: return p\n");
    // The checker already rejects the receiver; nothing may be recorded for it.
    assert!(check(&source).is_err());
}
