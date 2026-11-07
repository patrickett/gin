//! Result variants: `A where (..) or` then the next variant on the following line.
use ast::{ClauseItem, Item, SourceFile};

fn file(text: &str) -> SourceFile {
    parser::parse_strict(text).expect("parses")
}

fn binding(file: &SourceFile) -> &ast::Binding {
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("a binding");
    };
    binding
}

#[test]
fn variants_continue_on_the_next_line_after_a_trailing_or() {
    let file = file(
        "reserve(n in 0 to 9) Reserved where (n >= 0) or\n  AllocError where (n <= 9):\n    Reserved(count: n)\n",
    );
    let binding = binding(&file);
    assert_eq!(binding.alternatives.len(), 1);
    let first = binding.contract.as_ref().unwrap();
    assert!(first.value.open.is_some() && first.value.close.is_some());
    assert_eq!(first.value.items.len(), 1);
    assert!(binding.alternatives[0].value.contract.is_some());
    assert!(binding.value.is_some());
}

#[test]
fn a_variant_may_have_no_claims_and_there_may_be_several() {
    let file = file("f(n in 0 to 9) A or B or C where (n > 0):\n    n\n");
    let binding = binding(&file);
    assert_eq!(binding.alternatives.len(), 2);
    assert!(binding.contract.is_none());
    assert!(binding.alternatives[0].value.contract.is_none());
    assert!(binding.alternatives[1].value.contract.is_some());
}

#[test]
fn claims_in_parentheses_are_joined_by_and_or_comma() {
    let file = file("f(n in 0 to 9) A where (n > 0 and n < 9, n >= 2) or B:\n    n\n");
    let contract = binding(&file).contract.as_ref().unwrap();
    assert_eq!(contract.value.items.len(), 3);
    assert_eq!(contract.value.joiners.len(), 2);
    assert!(
        contract
            .value
            .items
            .iter()
            .all(|item| matches!(item.value, ClauseItem::Claim(_)))
    );
}

#[test]
fn an_unclosed_claim_group_is_a_diagnostic() {
    assert!(parser::parse_strict("f(n in 0 to 9) A where (n > 0 or B:\n    n\n").is_err());
}

#[test]
fn only_a_function_result_takes_variants() {
    assert!(parser::parse_strict("value A or B: 1\n").is_err());
}

#[test]
fn a_binding_in_parentheses_is_an_expression() {
    use ast::Expr;
    let file = file("f(n in 0 to 9) in 0 to 99: (m in 0 to 99: n + 1)\n");
    let value = &binding(&file).value.as_ref().unwrap().expression;
    let Expr::Group(inner) = &value.value else {
        panic!("a group");
    };
    assert!(matches!(inner.value, Expr::Bind(_)));
}

#[test]
fn then_and_else_on_deeper_lines_continue_the_conditional() {
    let source = "pick(n in 0 to 9) in 0 to 9:\n    return when n = 0\n           then 1\n           else 2\nlater := 2\n";
    let file = file(source);
    assert_eq!(file.items.len(), 2);
}

#[test]
fn an_else_at_the_when_statement_indent_is_not_a_continuation() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    when n = 0 then\n        return 1\n    else\n        return 2\n";
    let file = file(source);
    assert_eq!(file.items.len(), 1);
}

#[test]
fn an_is_in_an_earlier_method_body_does_not_make_a_later_header_a_union() {
    let source = "Box has\n    first(self ref) in 0 to 9:\n        return when self is\n            A then 1\n            B then 2\n\n    second(self ref) Reserved where (1 = 1) or\n      AllocError:\n        return Reserved()\n\n    #private\n    third(self ref) in 0 to 9: return 3\n";
    let file = file(source);
    assert_eq!(file.items.len(), 1);
}

#[test]
fn inline_is_an_attribute_hint_on_a_method_and_attributes_may_share_a_line() {
    let source = "Add(result) has\n    #infix('+'), #inline\n    add(self, rhs Self) result\n";
    let file = file(source);
    assert_eq!(file.items.len(), 1);
}
