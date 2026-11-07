//! `Variant(name Type)`: a payload field declared in place, in an `or` list or an inline result.
use ast::{Argument, Expr, Item, TypeBody};

fn variant_arguments(source: &str) -> Vec<Argument> {
    let file = parser::parse_strict(source).unwrap_or_else(|d| panic!("{source:?}: {d:?}"));
    let Item::Type(declaration) = &file.items[0].value else { panic!("type") };
    let TypeBody::Is(root) = &declaration.body.value else { panic!("is") };
    let Expr::Call { arguments, .. } = &root.value else { panic!("call: {root:?}") };
    arguments.value.iter().map(|argument| argument.value.clone()).collect()
}

#[test]
fn a_named_payload_is_a_typed_argument() {
    let arguments = variant_arguments("Result is Reserved(storage Vacant(x))\n");
    assert!(matches!(&arguments[0], Argument::Typed { name, .. } if name.value == "storage"));
}

#[test]
fn several_payloads_are_several_typed_arguments() {
    let arguments = variant_arguments("Result is Opened(session Session, other Other)\n");
    assert_eq!(arguments.len(), 2);
    assert!(matches!(&arguments[1], Argument::Typed { name, .. } if name.value == "other"));
}

#[test]
fn the_positional_payload_is_unchanged() {
    let arguments = variant_arguments("Result is Reserved(Vacant(x))\n");
    assert!(matches!(&arguments[0], Argument::Positional(_)));
}

#[test]
fn a_typed_payload_parses_in_an_inline_result_header() {
    let source = "f(n in 0 to 9) Reserved(storage Vacant(x)) where (storage.length = n) or Failed:\n    return Failed\n";
    parser::parse_strict(source).unwrap_or_else(|d| panic!("{d:?}"));
}

#[test]
fn a_value_call_still_rejects_two_adjacent_names() {
    // `f(a b)` is not `f(a: b)`; only a type may follow a name in place.
    assert!(parser::parse_strict("x: f(a b)\n").is_err());
    assert!(parser::parse_strict("x: f(a, b)\n").is_ok());
}

#[test]
fn a_payload_may_be_typed_by_a_type_parameter() {
    let arguments = variant_arguments("Result(element) is Failed(code element, detail element)\n");
    assert_eq!(arguments.len(), 2);
    assert!(matches!(&arguments[0], Argument::Typed { name, annotation } if name.value == "code" && matches!(&annotation.value, Expr::Name(n) if n == "element")));
}

#[test]
fn a_payload_may_be_typed_by_a_range() {
    let arguments = variant_arguments("Result is Aborted(proposal in 1 to 16, other Other)\n");
    assert!(matches!(&arguments[0], Argument::Typed { name, annotation } if name.value == "proposal" && matches!(annotation.value, Expr::Range { .. })));
    assert!(matches!(&arguments[1], Argument::Typed { .. }));
    let source = "f(n in 0 to 9) Done(count in 0 to 9) or Aborted(code element):\n    return Aborted(n)\n";
    parser::parse_strict(source).unwrap_or_else(|d| panic!("{d:?}"));
}

#[test]
fn a_value_call_still_rejects_a_name_then_a_range_or_a_lowercase_name() {
    assert!(parser::parse_strict("x: f(a in 1 to 2)\n").is_err());
    assert!(parser::parse_strict("x: f(a element)\n").is_err());
}
