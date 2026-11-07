//! Guards on case arms: `pattern and condition then value`.
use ast::{SyntaxNodeKind, Expr, Expression, Item, MatchExpression, MatchPattern, SourceFile, Statement};

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

fn case(expression: &Expression) -> &MatchExpression {
    let Expr::Match(case) = &expression.value else {
        panic!("case, found {:?}", expression.value)
    };
    case
}

const GUARDED: &str = "\
f(attempt Attempt) Outcome:
    return when attempt is
        Transferred(count) and count.as_num = 1 then Interrupted
        Transferred(count) then Done(count)
        Failed(error) then Broken(error)
later := 2
";

#[test]
fn an_arm_carries_a_guard_after_its_pattern() {
    let file = parser::parse_strict(GUARDED).unwrap();
    let case = case(returned(&file));
    let text = |span| file.span_table.get(span).extract(GUARDED);
    assert_eq!(case.arms.value.len(), 3);
    let first = &case.arms.value[0].value;
    let guard = first.guard.as_ref().expect("guard");
    assert_eq!(text(guard.and_keyword), "and");
    assert_eq!(text(guard.condition.span_id), "count.as_num = 1");
    assert_eq!(text(first.then_keyword), "then");
    assert_eq!(text(first.body.span_id), "Interrupted");
    assert!(matches!(first.pattern.value, MatchPattern::Variant(_)));
    assert_eq!(
        text(case.arms.value[0].span_id),
        "Transferred(count) and count.as_num = 1 then Interrupted"
    );
    // A guard-less arm for the same constructor sits beside the guarded one.
    assert!(case.arms.value[1].value.guard.is_none());
    assert!(case.arms.value[2].value.guard.is_none());
    assert_eq!(file.items.len(), 2);
}

#[test]
fn the_guard_may_use_connectives_and_is_tests() {
    let source = "\
f(a Attempt) Outcome:
    return when a is
        Pair(x, y) and x < y and not y = 3 then One
        Pair(x, y) and y is Ready or x = 1 then Two
        else Three
";
    let file = parser::parse_strict(source).unwrap();
    let case = case(returned(&file));
    let text = |span| file.span_table.get(span).extract(source);
    let first = case.arms.value[0].value.guard.as_ref().unwrap();
    assert_eq!(text(first.condition.span_id), "x < y and not y = 3");
    let second = case.arms.value[1].value.guard.as_ref().unwrap();
    assert_eq!(text(second.condition.span_id), "y is Ready or x = 1");
    assert!(matches!(case.arms.value[2].value.pattern.value, MatchPattern::Else { .. }));
}

#[test]
fn the_guard_keyword_and_condition_are_query_nodes() {
    let file = parser::parse_strict(GUARDED).unwrap();
    let and = file.node_at_offset(GUARDED.find(" and ").unwrap() + 1).unwrap();
    assert!(matches!(and.node.kind, SyntaxNodeKind::AndKeyword));
    let condition = file.node_at_offset(GUARDED.find("count.as_num").unwrap()).unwrap();
    assert!(matches!(condition.node.kind, SyntaxNodeKind::Name(_)));
}

#[test]
fn a_guard_without_then_recovers_before_later_declarations() {
    let source = "\
f(a Attempt) Outcome:
    return when a is
        Failed(error) and error.code = 1 Broken
later := 2
";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert!(output.file.declaration_outline().any(|entry| entry.name == "later"));
}

#[test]
fn integer_arms_keep_their_rules() {
    let source = "f(n Int) Int:\n    return when n is\n        1 and n > 0 then 2\n        else 3\n";
    assert!(parser::parse_strict(source).is_err());
}
