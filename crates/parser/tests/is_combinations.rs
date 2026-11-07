//! `x is Pattern` as an ordinary condition term, combined with `and`, `or` and `not`.
use ast::{BinaryOperator, Expr, Expression, Item, SourceFile, Statement};

fn returned(file: &SourceFile) -> &Expression {
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("function")
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("block body")
    };
    let Statement::Return(ret) = &block.statements.last().unwrap().value else {
        panic!("return")
    };
    ret.value.as_ref().unwrap()
}

fn binary(expression: &Expression, operator: BinaryOperator) -> (&Expression, &Expression) {
    let Expr::Binary {
        left,
        operator: found,
        right,
    } = &expression.value
    else {
        panic!("binary, found {:?}", expression.value)
    };
    assert_eq!(found.value, operator);
    (left, right)
}

fn is_test(expression: &Expression) {
    assert!(
        matches!(expression.value, Expr::IsTest(_)),
        "is test, found {:?}",
        expression.value
    );
}

fn condition(source: &str) -> (SourceFile, String) {
    let file = parser::parse_strict(source).unwrap_or_else(|error| panic!("{error:?}\n{source}"));
    (file, source.to_owned())
}

#[test]
fn one_line_conditional_combines_two_tests_with_and() {
    let source = "f(a Pick, b Pick) Int:\n    return when a is X and b is Y then 1 else 2\n";
    let (file, source) = condition(source);
    let Expr::Conditional(conditional) = &returned(&file).value else {
        panic!("conditional")
    };
    let (left, right) = binary(&conditional.condition, BinaryOperator::And);
    is_test(left);
    is_test(right);
    let text = |span| file.span_table.get(span).extract(&source);
    assert_eq!(text(left.span_id), "a is X");
    assert_eq!(text(right.span_id), "b is Y");
    assert_eq!(text(conditional.then_keyword), "then");
}

#[test]
fn a_test_on_the_left_of_or_with_a_comparison() {
    let source = "f(x Pick, y Int) Int:\n    return when x is P(v) or y < 3 then 1 else 2\n";
    let (file, _) = condition(source);
    let Expr::Conditional(conditional) = &returned(&file).value else {
        panic!("conditional")
    };
    let (left, right) = binary(&conditional.condition, BinaryOperator::Or);
    is_test(left);
    binary(right, BinaryOperator::Less);
}

#[test]
fn not_covers_a_parenthesized_test() {
    let source = "f(x Pick, y Int) Int:\n    return when not (x is P) and y < 3 then 1 else 2\n";
    let (file, _) = condition(source);
    let Expr::Conditional(conditional) = &returned(&file).value else {
        panic!("conditional")
    };
    let (left, _) = binary(&conditional.condition, BinaryOperator::And);
    assert!(matches!(left.value, Expr::Unary { .. }));
}

#[test]
fn nested_not_and_tests_in_the_right_operand() {
    let source = "f(x Pick, y Pick) Int:\n    return when x is P or not (y is Q and not (x is R)) then 1 else 2\n";
    let (file, _) = condition(source);
    let Expr::Conditional(conditional) = &returned(&file).value else {
        panic!("conditional")
    };
    let (left, right) = binary(&conditional.condition, BinaryOperator::Or);
    is_test(left);
    assert!(matches!(right.value, Expr::Unary { .. }));
}

#[test]
fn chain_arms_combine_tests() {
    let source = "\
f(a Pick, b Pick) Int:
    return when
        a is X and b is Y then 1
        a is Z or b is Z then 2
        not (a is X) then 3
        else 4
";
    let (file, _) = condition(source);
    let Expr::Chain(chain) = &returned(&file).value else {
        panic!("chain")
    };
    assert_eq!(chain.arms.len(), 3);
    let (left, right) = binary(&chain.arms[0].value.condition, BinaryOperator::And);
    is_test(left);
    is_test(right);
    let (left, right) = binary(&chain.arms[1].value.condition, BinaryOperator::Or);
    is_test(left);
    is_test(right);
    assert!(matches!(chain.arms[2].value.condition.value, Expr::Unary { .. }));
}

#[test]
fn when_statements_combine_tests() {
    let source = "\
f(a Pick, b Pick) Int:
    when a is X and b is Y then
        return 1
    when a is X(v) or b is Y then
        return 2
    return 3
";
    let (file, _) = condition(source);
    let Item::Binding(binding) = &file.items[0].value else {
        panic!()
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!()
    };
    let Statement::When(first) = &block.statements[0].value else {
        panic!("when statement, found {:?}", block.statements[0].value)
    };
    binary(&first.condition, BinaryOperator::And);
    let Statement::When(second) = &block.statements[1].value else {
        panic!("when statement")
    };
    binary(&second.condition, BinaryOperator::Or);
}

#[test]
fn a_single_variant_test_is_still_a_when_is_statement() {
    let source = "f(a Pick) Int:\n    when a is X(v) then\n        return 1\n    return 2\n";
    let (file, _) = condition(source);
    let Item::Binding(binding) = &file.items[0].value else {
        panic!()
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!()
    };
    assert!(matches!(block.statements[0].value, Statement::WhenIs(_)));
}

#[test]
fn a_comparison_subject_keeps_its_grouping() {
    let source = "f(x Int, y Int) Int:\n    return when x < y is True then 1 else 2\n";
    let (file, _) = condition(source);
    let Expr::Conditional(conditional) = &returned(&file).value else {
        panic!("conditional")
    };
    let Expr::IsTest(test) = &conditional.condition.value else {
        panic!("is test")
    };
    binary(&test.subject, BinaryOperator::Less);
}

#[test]
fn recovery_keeps_later_declarations() {
    let source = "f(a Pick) Int:\n    return when a is X and then 1 else 2\nlater := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert!(output.file.declaration_outline().any(|entry| entry.name == "later"));
}
