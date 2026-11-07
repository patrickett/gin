//! `when condition then value else other`: an inline branch on a condition.
use ast::{BinaryOperator, Expr, Expression, Item, SourceFile, Statement, SyntaxNodeKind};

fn body(file: &SourceFile) -> &ast::Block {
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("function")
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("block body")
    };
    block
}

fn returned(file: &SourceFile) -> &Expression {
    let Statement::Return(ret) = &body(file).statements.last().unwrap().value else {
        panic!("return")
    };
    ret.value.as_ref().unwrap()
}

#[test]
fn a_conditional_branches_on_a_comparison() {
    let source = "f(a in 0 to 9, b in 0 to 9) in 0 to 1:\n  return when a < b then 1 else 0\n";
    let file = parser::parse_strict(source).unwrap();
    let Expr::Conditional(case) = &returned(&file).value else {
        panic!("conditional")
    };
    let Expr::Binary { operator, .. } = &case.condition.value else {
        panic!("comparison")
    };
    assert_eq!(operator.value, BinaryOperator::Less);
    let text = |span| file.span_table.get(span).extract(source);
    assert_eq!(text(case.when_keyword), "when");
    assert_eq!(text(case.then_keyword), "then");
    assert_eq!(text(case.else_keyword), "else");
    assert_eq!(text(case.then_branch.span_id), "1");
    assert_eq!(text(case.else_branch.span_id), "0");
    assert_eq!(text(returned(&file).span_id), "when a < b then 1 else 0");
}

#[test]
fn a_conditional_is_a_value_anywhere_an_expression_is() {
    let source = "f(a in 0 to 9) in 0 to 1:\n  r: when a = 0 then 1 else 0\n  return r\n";
    let file = parser::parse_strict(source).unwrap();
    let Statement::Binding(local) = &body(&file).statements[0].value else {
        panic!("local")
    };
    let value = &local.value.as_ref().unwrap().expression;
    assert!(matches!(value.value, Expr::Conditional(_)));
}

#[test]
fn conditionals_nest_in_either_branch() {
    for source in [
        "f(a in 0 to 9) in 0 to 2:\n  return when a < 3 then 0 else when a < 6 then 1 else 2\n",
        "f(a in 0 to 9) in 0 to 2:\n  return when a < 6 then when a < 3 then 0 else 1 else 2\n",
    ] {
        let file = parser::parse_strict(source).expect(source);
        assert!(
            matches!(returned(&file).value, Expr::Conditional(_)),
            "{source}"
        );
    }
    let file = parser::parse_strict(
        "f(a in 0 to 9) in 0 to 2:\n  return when a < 3 then 0 else when a < 6 then 1 else 2\n",
    )
    .unwrap();
    let Expr::Conditional(outer) = &returned(&file).value else {
        panic!()
    };
    assert!(matches!(outer.else_branch.value, Expr::Conditional(_)));
    assert!(!matches!(outer.then_branch.value, Expr::Conditional(_)));
}

#[test]
fn the_branches_are_ordinary_expressions() {
    let source =
        "f(a in 0 to 9, b in 0 to 9) in 0 to 18:\n  return when a < b then a + b else b * 2\n";
    let file = parser::parse_strict(source).unwrap();
    let Expr::Conditional(case) = &returned(&file).value else {
        panic!()
    };
    assert!(matches!(case.then_branch.value, Expr::Binary { .. }));
    assert!(matches!(case.else_branch.value, Expr::Binary { .. }));
}

#[test]
fn a_conditional_without_else_or_then_is_a_syntax_error() {
    for source in [
        "f(a in 0 to 9) in 0 to 1:\n  return when a < 3 then 1\n",
        "f(a in 0 to 9) in 0 to 1:\n  return when a < 3 then\n",
        "f(a in 0 to 9) in 0 to 1:\n  return when a < 3 else 1\n",
        "f(a in 0 to 9) in 0 to 1:\n  return when a < 3 then 1 else\n",
    ] {
        assert!(parser::parse_strict(source).is_err(), "{source:?}");
    }
}

#[test]
fn a_case_over_patterns_is_unchanged() {
    let source = "f(a in 0 to 1) in 0 to 1:\n  return when a is\n    0 then 1\n    1 then 0\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(returned(&file).value, Expr::Match(_)));
}

#[test]
fn the_syntax_query_exposes_the_conditional_and_its_keywords() {
    let source = "f(a in 0 to 9) in 0 to 1:\n  return when a < 3 then 1 else 0\n";
    let file = parser::parse_strict(source).unwrap();
    let kind = |word: &str| {
        file.node_at_offset(source.find(word).unwrap())
            .unwrap()
            .node
            .kind
    };
    assert!(matches!(kind("when"), SyntaxNodeKind::WhenKeyword));
    assert!(matches!(kind("then"), SyntaxNodeKind::ThenKeyword));
    assert!(matches!(kind("else"), SyntaxNodeKind::ElseKeyword));
}
