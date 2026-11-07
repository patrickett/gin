//! `where P` after a loop head is the loop's invariant: kept as written, optional, one per loop.
use ast::{Expr, Item, LoopExpression, Statement};

fn loops(text: &str) -> Vec<LoopExpression> {
    let file = parser::parse_strict(text).expect("parses");
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("binding")
    };
    let body = &binding.value.as_ref().unwrap().expression;
    let Expr::Block(block) = &body.value else {
        panic!("block")
    };
    block
        .statements
        .iter()
        .filter_map(|statement| match &statement.value {
            Statement::Expression(expression) => match &expression.value {
                Expr::Loop(found) => Some((**found).clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn a_while_and_a_range_for_keep_their_written_invariant() {
    let found = loops("f(n):\n    while i < n where a = i do\n        a :: a + 1\n    for j in 0 to n where a = j do\n        a :: a + 1\n    return\n");
    assert_eq!(found.len(), 2);
    let LoopExpression::While { invariant, .. } = &found[0] else { panic!("while") };
    assert!(invariant.is_some());
    let LoopExpression::For { invariant, .. } = &found[1] else { panic!("for") };
    assert!(invariant.is_some());
}

#[test]
fn an_invariant_is_optional_and_none_is_invented() {
    let found = loops("f(n):\n    while i < n do\n        a :: a + 1\n    for j in 0 to n do\n        a :: a + 1\n    return\n");
    for loop_expression in &found {
        match loop_expression {
            LoopExpression::While { invariant, .. } | LoopExpression::For { invariant, .. } => {
                assert!(invariant.is_none())
            }
        }
    }
}

#[test]
fn a_where_without_a_condition_or_a_second_where_is_refused() {
    assert!(parser::parse_strict("f(n):\n    while i < n where do\n        a :: a + 1\n    return\n").is_err());
    assert!(parser::parse_strict("f(n):\n    while i < n where a = i where b = i do\n        a :: a + 1\n    return\n").is_err());
}
