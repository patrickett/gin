use ast::{BindValue, Condition, Expr, InRangeBounds, Pattern};
use parser::query::SourceParseExt;

fn parse_when(condition: &str) -> Condition {
    let source = format!("main:\n    when {condition} then\n        1\n        return 0\n    return 0\n");
    let out = source.as_str().parse_source_full();
    assert!(
        out.symptoms.is_empty(),
        "unexpected parse symptoms: {:?}",
        out.symptoms
    );

    let bind = out
        .ast
        .defs
        .get(&internment::Intern::from_ref("main"))
        .expect("main should exist");

    let Expr::When(when_expr) = (match &bind.value {
        BindValue::Body { exprs, .. } if !exprs.is_empty() => &exprs[0].value,
        BindValue::Expr(expr) => &expr.value,
        _ => panic!("main should have an if expression"),
    }) else {
        panic!("first expression in main should be an if expr");
    };
    match &when_expr.arms[0] {
        ast::WhenArm::Cond { condition, .. } => condition.clone(),
        ast::WhenArm::Is { pattern, .. } => Condition::Is {
            subject: when_expr.subject.clone().unwrap(), pattern: pattern.clone(),
        },
        _ => panic!(),
    }
}

#[test]
fn parse_if_match_condition_is_structural_leaf() {
    assert!(matches!(parse_when("1 is 1"), Condition::Is { .. }));
}

#[test]
fn parse_if_range_preserves_literal_lower_and_symbolic_upper_bounds() {
    assert!(matches!(
        parse_when("value is in 1 to max_alignment"),
        Condition::Is { pattern, .. }
            if matches!(
                pattern.value,
                Pattern::InRange {
                    bounds: InRangeBounds::LiteralToTag(min, max),
                    ..
                } if min == 1.into() && max.as_str() == "max_alignment"
            )
    ));
}

#[test]
fn parse_if_and_condition_preserves_both_clauses() {
    assert!(matches!(
        parse_when("1 is 1 and 2 is 2"),
        Condition::And(left, right)
            if matches!(*left, Condition::Is { .. })
                && matches!(*right, Condition::Is { .. })
    ));
}

#[test]
fn parse_if_not_binds_tighter_than_and() {
    assert!(matches!(
        parse_when("1 is 1 and not 2 is 3"),
        Condition::And(_, right) if matches!(*right, Condition::Not(_))
    ));
}

#[test]
fn parse_if_and_binds_tighter_than_or() {
    assert!(matches!(
        parse_when("1 is 1 or 2 is 2 and 3 is 3"),
        Condition::Or(_, right) if matches!(*right, Condition::And(_, _))
    ));
}

#[test]
fn parse_if_parentheses_override_condition_precedence() {
    assert!(matches!(
        parse_when("(1 is 1 or 2 is 2) and 3 is 3"),
        Condition::And(left, _) if matches!(*left, Condition::Or(_, _))
    ));
}

#[test]
fn parse_if_without_pattern_accepts_a_proposition_expression() {
    let source = "main:\n    when 1 then\n        1\n        return 0\n    return 0\n";
    let out = source.parse_source_full();
    assert!(out.symptoms.is_empty(), "{:?}", out.symptoms);
}

#[test]
fn numeric_predicates_share_refinement_relations() {
    for (operator, relation) in [
        ("=", ast::ProofRelation::Equal),
        ("<", ast::ProofRelation::Less),
        ("<=", ast::ProofRelation::LessOrEqual),
        (">", ast::ProofRelation::Greater),
        (">=", ast::ProofRelation::GreaterOrEqual),
    ] {
        assert!(
            matches!(parse_when(&format!("value {operator} limit")), Condition::Compare { relation: actual, .. } if actual == relation)
        );
    }
    assert!(
        matches!(parse_when("value in 0 to 255"), Condition::Is { pattern, .. } if matches!(pattern.value, Pattern::InRange { .. }))
    );
    assert!(
        matches!(parse_when("value < limit or value = 0 and value in 0 to 255"), Condition::Or(_, right) if matches!(*right, Condition::And(_, _)))
    );
}
