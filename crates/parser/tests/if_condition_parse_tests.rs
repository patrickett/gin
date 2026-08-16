use ast::{BindValue, Condition, Expr, InRangeBounds, Pattern};
use parser::query::SourceParseExt;

fn parse_if(condition: &str) -> ast::IfExpr {
    let source = format!(
        "main:\n\
    if {condition}\n\
        1\n\
    return 0\n\
"
    );
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

    let Expr::If(if_expr) = (match &bind.value {
        BindValue::Body { exprs, .. } if !exprs.is_empty() => &exprs[0].value,
        BindValue::Expr(expr) => &expr.value,
        _ => panic!("main should have an if expression"),
    }) else {
        panic!("first expression in main should be an if expr");
    };
    if_expr.clone()
}

#[test]
fn parse_if_match_condition_is_structural_leaf() {
    assert!(matches!(parse_if("1 is 1").condition, Condition::Is { .. }));
}

#[test]
fn parse_if_range_preserves_literal_lower_and_symbolic_upper_bounds() {
    assert!(matches!(
        parse_if("value is in 1...max_alignment").condition,
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
        parse_if("1 is 1 and 2 is 2").condition,
        Condition::And(left, right)
            if matches!(*left, Condition::Is { .. })
                && matches!(*right, Condition::Is { .. })
    ));
}

#[test]
fn parse_if_not_binds_tighter_than_and() {
    assert!(matches!(
        parse_if("1 is 1 and not 2 is 3").condition,
        Condition::And(_, right) if matches!(*right, Condition::Not(_))
    ));
}

#[test]
fn parse_if_and_binds_tighter_than_or() {
    assert!(matches!(
        parse_if("1 is 1 or 2 is 2 and 3 is 3").condition,
        Condition::Or(_, right) if matches!(*right, Condition::And(_, _))
    ));
}

#[test]
fn parse_if_parentheses_override_condition_precedence() {
    assert!(matches!(
        parse_if("(1 is 1 or 2 is 2) and 3 is 3").condition,
        Condition::And(left, _) if matches!(*left, Condition::Or(_, _))
    ));
}

#[test]
fn parse_if_without_pattern_reports_explicit_pattern_requirement() {
    let source = "\
main:\n\
    if 1\n\
        1\n\
    return 0\n\
";
    let out = source.parse_source_full();
    assert!(
        out.symptoms
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "parse-expected-condition-pattern"),
        "symptoms: {:?}",
        out.symptoms
    );
}
