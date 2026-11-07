//! `not` and `is` in conditions. `not` covers the comparison or test after it, up to `and` or
//! `or`; `subject is Pattern` is a test with the precedence of a comparison.
use ast::{BinaryOperator, Expr, Item, SourceFile, Statement, UnaryOperator};

fn value(file: &SourceFile) -> &ast::Expression {
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("binding")
    };
    &binding.value.as_ref().unwrap().expression
}

fn parse(source: &str) -> ast::Expression {
    let file = parser::parse_strict(source).unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
    value(&file).clone()
}

fn conditional(source: &str) -> ast::ConditionalExpression {
    let Expr::Conditional(case) = parse(source).value else {
        panic!("conditional in {source:?}")
    };
    *case
}

#[test]
fn not_negates_a_parenthesized_comparison() {
    let case = conditional("x: when not (a = b) then 1 else 0\n");
    let Expr::Unary { operator, operand } = &case.condition.value else {
        panic!("not")
    };
    assert_eq!(operator.value, UnaryOperator::Not);
    assert!(matches!(operand.value, Expr::Group(_)));
}

/// `not` covers the whole comparison after it, not just its first operand.
#[test]
fn not_covers_a_bare_comparison() {
    let case = conditional("x: when not a < b then 1 else 0\n");
    let Expr::Unary { operator, operand } = &case.condition.value else {
        panic!("not")
    };
    assert_eq!(operator.value, UnaryOperator::Not);
    let Expr::Binary { operator, .. } = &operand.value else {
        panic!("comparison under not: {operand:?}")
    };
    assert_eq!(operator.value, BinaryOperator::Less);
}

/// `not` stops before `and` and `or`.
#[test]
fn not_stops_at_and_and_or() {
    let case = conditional("x: when a = b and not c = d or e = f then 1 else 0\n");
    let Expr::Binary { operator, left, .. } = &case.condition.value else {
        panic!("or")
    };
    assert_eq!(operator.value, BinaryOperator::Or);
    let Expr::Binary {
        operator, right, ..
    } = &left.value
    else {
        panic!("and")
    };
    assert_eq!(operator.value, BinaryOperator::And);
    let Expr::Unary { operand, .. } = &right.value else {
        panic!("not under and: {right:?}")
    };
    assert!(matches!(operand.value, Expr::Binary { .. }));
}

#[test]
fn is_tests_a_number_or_a_member() {
    for source in [
        "x: when count is 0 then 1 else 2\n",
        "x: when state is Active then 1 else 2\n",
        "x: when darwin is True then 72 else 36\n",
    ] {
        let case = conditional(source);
        assert!(matches!(case.condition.value, Expr::IsTest(_)), "{source:?}");
    }
}

#[test]
fn not_covers_an_is_test() {
    let source = "main:\n    while not remaining is 0 do\n        step()\n    return 0\n";
    let file = parser::parse_strict(source).unwrap();
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("binding")
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("block")
    };
    let Statement::Expression(expression) = &block.statements[0].value else {
        panic!("loop")
    };
    let Expr::Loop(looped) = &expression.value else {
        panic!("loop expression")
    };
    let ast::LoopExpression::While { condition, .. } = &**looped else {
        panic!("while")
    };
    let Expr::Unary { operator, operand } = &condition.value else {
        panic!("not: {condition:?}")
    };
    assert_eq!(operator.value, UnaryOperator::Not);
    assert!(matches!(operand.value, Expr::IsTest(_)));
}

/// The statement forms are unchanged: a one-armed case and a full case.
#[test]
fn is_after_when_still_starts_a_case() {
    let one = "f(r Result) in 0 to 1:\n    when r is Failed(_) then return 1\n    return 0\n";
    let file = parser::parse_strict(one).unwrap();
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("binding")
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("block")
    };
    assert!(matches!(block.statements[0].value, Statement::WhenIs(_)));
    let full = "f(r Result) in 0 to 1: when r is\n    Failed(_) then 1\n    Done then 0\n";
    assert!(matches!(parse(full).value, Expr::Match(_)));
}

#[test]
fn a_not_with_nothing_after_it_is_an_error() {
    assert!(parser::parse_strict("x: when not then 1 else 0\n").is_err());
}

// A case arm whose body is `return`.

fn arms(source: &str) -> Vec<ast::MatchArm> {
    let file = parser::parse_strict(source).unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("binding")
    };
    let mut expression = &binding.value.as_ref().unwrap().expression;
    // The case may be the body, or the one statement of a block body.
    if let Expr::Block(block) = &expression.value
        && let Statement::Expression(inner) = &block.statements[0].value
    {
        expression = inner;
    }
    let Expr::Match(case) = &expression.value else {
        panic!("case in {source:?}: {:?}", expression.value)
    };
    case.arms.value.iter().map(|arm| arm.value.clone()).collect()
}

#[test]
fn an_arm_may_return_on_its_own_line() {
    let arms = arms("release(self):\n    when self is\n        Pushed then return\n        Failed(_) then return 1\n");
    for arm in arms {
        let Expr::Block(block) = &arm.body.value else {
            panic!("a returning arm is a block: {:?}", arm.body)
        };
        assert!(matches!(block.statements[0].value, Statement::Return(_)));
    }
}

#[test]
fn an_arm_may_return_beside_arms_that_have_values() {
    let arms = arms("f(self) in 0 to 9: when self is\n    Copied(value) then value\n    AllocError(error) then return error\n");
    assert!(matches!(arms[0].body.value, Expr::Name(_)));
    assert!(matches!(arms[1].body.value, Expr::Block(_)));
}

// Compile-time queries and `domain of`.

fn value_of(source: &str) -> ast::Expression {
    let file = parser::parse_strict(source).unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
    value(&file).clone()
}

#[test]
fn a_query_is_a_name_after_a_hash() {
    let query = value_of("x: #size(element)\n");
    let Expr::Call { callee, .. } = &query.value else {
        panic!("call: {query:?}")
    };
    assert!(matches!(&callee.value, Expr::Query { name, .. } if name.value == "size"));
}

#[test]
fn a_query_may_have_a_member() {
    let query = value_of("x: #target.triple\n");
    let Expr::Member { base, name } = &query.value else {
        panic!("member: {query:?}")
    };
    assert!(matches!(&base.value, Expr::Query { name, .. } if name.value == "target"));
    assert_eq!(name.value, "triple");
}

#[test]
fn the_hole_is_still_a_hole() {
    assert!(matches!(value_of("x: #todo\n").value, Expr::Hole));
}

#[test]
fn domain_of_is_a_prefix_operator_and_a_group_takes_the_member() {
    // Like unary minus it binds after member access, so a member of the range is written on a
    // group, as the draft does.
    let bounds = value_of("x: (domain of Byte).max\n");
    let Expr::Member { base, name } = &bounds.value else {
        panic!("member: {bounds:?}")
    };
    assert_eq!(name.value, "max");
    let Expr::Group(inner) = &base.value else {
        panic!("group: {base:?}")
    };
    let Expr::Unary { operator, operand } = &inner.value else {
        panic!("domain of: {inner:?}")
    };
    assert_eq!(operator.value, UnaryOperator::DomainOf);
    assert!(matches!(&operand.value, Expr::Name(name) if name == "Byte"));
}

// Domain membership: `x is in T`.

#[test]
fn is_in_tests_membership_of_a_domain() {
    for source in [
        "x: when result is in SignedBigInt then 1 else 0\n",
        "x: when value is in -128 to 127 then 1 else 0\n",
        "x: when next is in Count(element) then 1 else 0\n",
    ] {
        let case = conditional(source);
        let Expr::IsTest(test) = &case.condition.value else {
            panic!("test in {source:?}: {:?}", case.condition.value)
        };
        assert!(
            matches!(test.pattern.value, ast::MatchPattern::Domain { .. }),
            "{source:?}"
        );
    }
}

/// The domain stops at `and`/`or`, so a membership can be one side of a condition. After an
/// operator, `is` is a test inside the condition, not the end of a case subject.
#[test]
fn a_membership_may_be_joined_to_other_conditions() {
    for source in [
        "x: when b > 2 and a is in Byte then 1 else 0\n",
        "x: when (a is in Byte) and b > 2 then 1 else 0\n",
    ] {
        let case = conditional(source);
        let Expr::Binary { operator, .. } = &case.condition.value else {
            panic!("and in {source:?}: {:?}", case.condition.value)
        };
        assert_eq!(operator.value, BinaryOperator::And, "{source:?}");
    }
    let case = conditional("x: when b > 2 and a is in Byte then 1 else 0\n");
    let Expr::Binary { right, .. } = &case.condition.value else {
        panic!("and")
    };
    assert!(matches!(right.value, Expr::IsTest(_)), "{right:?}");
}

/// A test on an expression with an operator is a condition, never a case.
#[test]
fn is_after_an_arithmetic_subject_is_a_test() {
    let case = conditional("x: when a + 1 is 0 then 1 else 0\n");
    let Expr::IsTest(test) = &case.condition.value else {
        panic!("test: {:?}", case.condition.value)
    };
    assert!(matches!(test.subject.value, Expr::Binary { .. }));
}

#[test]
fn a_membership_statement_is_a_condition_not_a_case() {
    let source = "f(n in 0 to 99) in 0 to 1:\n    when n is in 0 to 9 then return 1\n    return 0\n";
    let file = parser::parse_strict(source).unwrap();
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("binding")
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("block")
    };
    assert!(matches!(block.statements[0].value, Statement::When(_)));
}

#[test]
fn not_covers_a_membership() {
    let case = conditional("x: when not a is in Byte then 1 else 0\n");
    let Expr::Unary { operator, operand } = &case.condition.value else {
        panic!("not")
    };
    assert_eq!(operator.value, UnaryOperator::Not);
    assert!(matches!(operand.value, Expr::IsTest(_)));
}

// Text arms and the default arm.

#[test]
fn a_case_over_text_has_text_arms_and_an_else_arm() {
    let source = "name(): when label() is\n    'putchar' then 1\n    'labs' then 2\n    else 3\n";
    let arms = arms(source);
    assert_eq!(arms.len(), 3);
    assert!(matches!(arms[0].pattern.value, ast::MatchPattern::Text(_)));
    assert!(matches!(arms[1].pattern.value, ast::MatchPattern::Text(_)));
    assert!(matches!(arms[2].pattern.value, ast::MatchPattern::Else { .. }));
}

#[test]
fn an_else_arm_takes_a_block_or_a_value_on_the_same_line() {
    for source in [
        "f(): when label() is\n    'a' then 1\n    else 2\n",
        "f(): when label() is\n    'a' then 1\n    else\n        2\n",
    ] {
        let arms = arms(source);
        assert!(matches!(arms[1].pattern.value, ast::MatchPattern::Else { .. }), "{source}");
    }
}

/// `else then value` is still not an arm: the default has no `then`.
#[test]
fn an_else_arm_has_no_then() {
    assert!(parser::parse_strict("f(): when label() is\n    'a' then 1\n    else then 2\n").is_err());
}
