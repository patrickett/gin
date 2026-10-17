use super::*;

#[test]
fn arithmetic_fallback_rejects_an_unproved_lower_bound() {
    let local = NormalExpr::Var(Intern::from_ref("old"));
    let mut constraints = ConstraintEnv::default();
    constraints.assume(&Predicate::Ge(local.clone(), 1.into()));
    constraints.assume(&Predicate::Le(local.clone(), 10.into()));
    assert!(!rebind_rhs_within_integer_bounds(
        NormalExpr::Add(Box::new(local.clone()), Box::new((-1).into())),
        local,
        &HashMap::new(),
        1,
        10,
        false,
        &constraints,
    ));
}

#[test]
fn arithmetic_fallback_accepts_the_inclusive_upper_bound() {
    let local = NormalExpr::Var(Intern::from_ref("old"));
    let mut constraints = ConstraintEnv::default();
    constraints.assume(&Predicate::Eq(local.clone(), 9.into()));
    assert!(rebind_rhs_within_integer_bounds(
        NormalExpr::Add(Box::new(local.clone()), Box::new(1.into())),
        local,
        &HashMap::new(),
        1,
        10,
        false,
        &constraints,
    ));
}

#[test]
fn arithmetic_fallback_does_not_overflow_a_wide_threshold() {
    let local = NormalExpr::Var(Intern::from_ref("old"));
    let mut constraints = ConstraintEnv::default();
    constraints.assume(&Predicate::Ge(local.clone(), 0.into()));
    assert!(!rebind_rhs_within_integer_bounds(
        NormalExpr::Add(Box::new(local.clone()), Box::new(i128::MIN.into())),
        local,
        &HashMap::new(),
        i128::MIN,
        i128::MAX,
        false,
        &constraints,
    ));
}

#[test]
fn range_facts_refer_to_the_captured_subject() {
    use crate::transform::{TransformCtx, transform};
    use crate::{BindBody, DefId, FileId};
    use parser::cursor::TokenCursor;

    let parsed = TokenCursor::parse_source(
        "inspect(value is in 0 to 255):\n    when value is in 1 to 10 then\n        1\n        return 0\n    return 0\n",
    );
    let typed = transform(&parsed, FileId(0), &TransformCtx::new());
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let BindBody::Body { exprs, .. } = &typed.defs[&DefId::named(Intern::from_ref("inspect"))].body
    else {
        panic!("expected body");
    };
    let TypedExprKind::When(expression) = &typed.exprs.kind[exprs[0].index()] else {
        panic!("expected when expression");
    };
    let TypedWhenArm::Is { pattern, .. } = &expression.arms[0] else {
        panic!("range arm")
    };
    let condition = crate::typed::TypedCondition::Is {
        subject: expression.subject.unwrap(),
        pattern: pattern.clone(),
    };
    let crate::typed::TypedCondition::Is { subject, .. } = &condition else {
        panic!("expected range condition");
    };
    let captured = NormalExpr::Var(Intern::from_ref("old_value"));
    let values = HashMap::from([(*subject, captured.clone())]);
    let facts = |taken, values| {
        condition_comparison_facts(
            &condition,
            taken,
            &typed,
            None,
            &HashMap::new(),
            None,
            Some(values),
        )
    };
    assert_eq!(
        facts(true, &values),
        vec![
            (1.into(), captured.clone(), false),
            (captured, 10.into(), false)
        ]
    );
    assert!(facts(false, &values).is_empty());
    assert!(facts(true, &HashMap::new()).is_empty());
}

#[test]
fn doubling_a_positive_divisor_stays_inside_the_interval() {
    let local = NormalExpr::Var(Intern::from_ref("old"));
    let mut constraints = ConstraintEnv::default();
    constraints.assume(&Predicate::Ge(local.clone(), 1.into()));
    constraints.assume(&Predicate::Lt(local.clone(), 16.into()));
    assert!(rebind_rhs_within_integer_bounds(
        NormalExpr::Mul(Box::new(local.clone()), Box::new(2.into())),
        local,
        &HashMap::new(),
        1,
        16,
        true,
        &constraints,
    ));
}

#[test]
fn doubling_still_needs_the_lower_bound() {
    let local = NormalExpr::Var(Intern::from_ref("old"));
    let mut constraints = ConstraintEnv::default();
    constraints.assume(&Predicate::Ge(local.clone(), 1.into()));
    constraints.assume(&Predicate::Lt(local.clone(), 16.into()));
    assert!(!rebind_rhs_within_integer_bounds(
        NormalExpr::Mul(Box::new(local.clone()), Box::new(2.into())),
        local,
        &HashMap::new(),
        3,
        16,
        true,
        &constraints,
    ));
}
