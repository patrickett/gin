//! `when` alone on a line, then guarded `condition then value` arms and a final `else`.
use ast::{ChainExpression, Expr, Expression, Item, MatchPattern, SourceFile, Statement};

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

fn chain(expression: &Expression) -> &ChainExpression {
    let Expr::Chain(chain) = &expression.value else {
        panic!("chain, found {:?}", expression.value)
    };
    chain
}

const GUARDS: &str = "\
f(status in 0 to 9) in 0 to 3:
    return when
        status < 0 then 1
        status = 4 and not status = 5 then 2
        else 3
";

#[test]
fn guard_arms_end_in_a_required_else() {
    let file = parser::parse_strict(GUARDS).unwrap();
    let chain = chain(returned(&file));
    let text = |span| file.span_table.get(span).extract(GUARDS);
    assert_eq!(chain.arms.len(), 2);
    assert_eq!(text(chain.when_keyword), "when");
    assert_eq!(text(chain.arms[0].value.condition.span_id), "status < 0");
    assert_eq!(text(chain.arms[0].value.then_keyword), "then");
    assert_eq!(text(chain.arms[0].value.value.span_id), "1");
    assert_eq!(text(chain.arms[1].span_id), "status = 4 and not status = 5 then 2");
    assert_eq!(text(chain.else_keyword), "else");
    assert_eq!(text(chain.otherwise.span_id), "3");
    assert!(text(returned(&file).span_id).starts_with("when\n"));
    assert!(text(returned(&file).span_id).ends_with("else 3"));
}

#[test]
fn then_may_follow_on_the_next_line_level_or_deeper() {
    let source = "\
f(r Result) Out:
    return when
        r is Failed(error)
        then Wrapped(error)
        r is Other
            then Deeper
        else Fine
";
    let file = parser::parse_strict(source).unwrap();
    let chain = chain(returned(&file));
    assert_eq!(chain.arms.len(), 2);
    for arm in &chain.arms {
        let Expr::IsTest(test) = &arm.value.condition.value else {
            panic!("is test")
        };
        assert!(matches!(test.pattern.value, MatchPattern::Variant(_)));
    }
    let text = |span| file.span_table.get(span).extract(source);
    assert_eq!(text(chain.arms[0].value.condition.span_id), "r is Failed(error)");
    assert_eq!(text(chain.arms[0].value.value.span_id), "Wrapped(error)");
    assert_eq!(text(chain.arms[1].value.value.span_id), "Deeper");
}

#[test]
fn a_chain_is_a_binding_value() {
    let source = "\
f(a in 0 to 9) in 0 to 2:
    r := when
        a = 0 then 1
        else 2
    later := 2
    return r
";
    let file = parser::parse_strict(source).unwrap();
    let Statement::Binding(local) = &body(&file).statements[0].value else {
        panic!("local")
    };
    chain(&local.value.as_ref().unwrap().expression);
    assert_eq!(body(&file).statements.len(), 3);
}

#[test]
fn a_chain_as_a_method_body_on_the_header_line() {
    let source = "\
T has
    from_num(value in R) Self: return when
        value = -4 then Interrupted
        value < 0 then Failed(value as R)
        else Done(value as R)

    other(self) in R: return when self is
        Done(count) then count
        Interrupted then -4
        Failed(code) then code
";
    let output = parser::parse(source).unwrap();
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
}

#[test]
fn missing_else_is_a_diagnostic() {
    let source = "f(a in 0 to 9) in 0 to 2:\n    r := when\n        a = 0 then 1\n    later := 2\n    return r\n";
    let output = parser::parse(source).unwrap();
    assert!(!output.diagnostics.is_empty());
    let Statement::Binding(local) = &body(&output.file).statements[0].value else {
        panic!("local")
    };
    let chain = chain(&local.value.as_ref().unwrap().expression);
    assert_eq!(chain.arms.len(), 1);
    assert!(matches!(chain.otherwise.value, Expr::Missing));
    assert!(output.file.span_table.get(chain.else_keyword).is_empty());
    assert_eq!(body(&output.file).statements.len(), 3);
}

#[test]
fn an_arm_without_then_is_a_diagnostic_and_later_arms_survive() {
    let source = "f(a in 0 to 9) in 0 to 3:\n    r := when\n        a = 0\n        a = 1 then 2\n        else 3\n    later := 2\n    return r\n";
    let output = parser::parse(source).unwrap();
    assert!(!output.diagnostics.is_empty());
    let Statement::Binding(local) = &body(&output.file).statements[0].value else {
        panic!("local")
    };
    let chain = chain(&local.value.as_ref().unwrap().expression);
    assert_eq!(chain.arms.len(), 2);
    assert!(matches!(chain.arms[0].value.value.value, Expr::Missing));
    assert!(matches!(chain.otherwise.value, Expr::Literal(_)));
    assert_eq!(body(&output.file).statements.len(), 3);
}

#[test]
fn a_garbled_arm_does_not_swallow_later_items() {
    let source = "f(a in 0 to 9) in 0 to 3:\n    r := when\n        a = ) then 1\n        a = 1 then 2\n        else 3\n    later := 2\n    return r\n\ng := 1\n";
    let output = parser::parse(source).unwrap();
    assert!(!output.diagnostics.is_empty());
    let statements = &body(&output.file).statements;
    let Statement::Binding(later) = &statements[1].value else {
        panic!("later binding survives")
    };
    assert_eq!(later.name.value, "later");
    assert_eq!(output.file.items.len(), 2);
}

#[test]
fn nothing_may_follow_the_else_arm() {
    let source = "f(a in 0 to 9) in 0 to 3:\n    r := when\n        a = 0 then 1\n        else 3\n        a = 1 then 2\n    later := 2\n    return r\n";
    let output = parser::parse(source).unwrap();
    assert!(!output.diagnostics.is_empty());
    assert_eq!(body(&output.file).statements.len(), 3);
}

#[test]
fn one_line_conditionals_and_subject_cases_are_unchanged() {
    let source = "f(a in 0 to 9) in 0 to 1:\n  return when a < 3 then 1 else 0\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(returned(&file).value, Expr::Conditional(_)));
    let source = "f(r Result) in 0 to 1:\n  return when r is Failed(e) then 1 else 0\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(returned(&file).value, Expr::Conditional(_)));
    let source = "f(r Result) in 0 to 1:\n  return when r is\n    Failed(e) then 1\n    Fine then 0\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(returned(&file).value, Expr::Match(_)));
}

#[test]
fn every_chain_token_is_a_query_child_in_source_order() {
    let file = parser::parse_strict(GUARDS).unwrap();
    let offset = |needle: &str| GUARDS.find(needle).unwrap();
    let kind = |needle: &str| {
        format!("{:?}", file.node_at_offset(offset(needle)).unwrap().node.kind)
    };
    assert_eq!(kind("when"), "WhenKeyword");
    assert_eq!(kind("then 1"), "ThenKeyword");
    assert_eq!(kind("else"), "ElseKeyword");
    let hit = file.node_at_offset(offset("then 2")).unwrap();
    assert!(hit.ancestors.iter().any(|node| format!("{:?}", node.kind) == "Chain"));
}
