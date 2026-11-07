//! `where place :: value`: what a call does to the places it is lent `mut`.
use ast::{BinaryOperator, ClauseItem, Expr, Item};

fn binding(source: &str) -> ast::Binding {
    let file = parser::parse_strict(source).unwrap();
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("function")
    };
    (**binding).clone()
}

#[test]
fn a_header_names_what_the_call_does_to_a_mut_place() {
    let source = "reset(x mut in 0 to 9) where x :: 0:\n  x :: 0\n  return\n";
    let function = binding(source);
    let contract = function.contract.expect("contract");
    assert_eq!(contract.value.items.len(), 1);
    let file = parser::parse_strict(source).unwrap();
    let ClauseItem::Transition(transition) = &contract.value.items[0].value else {
        panic!("transition")
    };
    assert_eq!(file.span_table.get(contract.value.where_keyword).extract(source), "where");
    assert_eq!(file.span_table.get(transition.operator).extract(source), "::");
    assert!(matches!(&transition.place.value, Expr::Name(name) if name == "x"));
    assert_eq!(file.span_table.get(transition.value.span_id).extract(source), "0");
    assert!(function.value.is_some(), "the body still follows the contract");
}

#[test]
fn several_transitions_over_fields_and_after_a_result() {
    let source = "step(p mut P, by in 0 to 3) in 0 to 9 where p.x :: p.x + by, p.y :: p.y, by :: by:\n  return by\n";
    let function = binding(source);
    assert!(function.annotation.is_some());
    let contract = function.contract.expect("contract");
    assert_eq!(contract.value.items.len(), 3);
    let ClauseItem::Transition(first) = &contract.value.items[0].value else {
        panic!("transition")
    };
    assert!(matches!(&first.place.value, Expr::Member { .. }));
    assert!(matches!(
        &first.value.value,
        Expr::Binary { operator, .. } if operator.value == BinaryOperator::Add
    ));
}

#[test]
fn a_function_without_a_contract_has_none() {
    assert!(binding("f(x in 0 to 9): return\n").contract.is_none());
}

#[test]
fn a_bare_expression_is_a_claim_not_a_transition_and_an_empty_clause_is_an_error() {
    let function = binding("f(x mut) where x + 1:\n  return\n");
    let contract = function.contract.expect("contract");
    assert!(matches!(
        contract.value.items[0].value,
        ClauseItem::Claim(ast::RefinementPredicate::Expression(_))
    ));
    assert!(parser::parse_strict("f(x mut) where:\n  return\n").is_err());
}
