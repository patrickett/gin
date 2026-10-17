use ast::{ConstValue, NormalExpr};
use i256::I256;
use std::collections::HashMap;
use typecheck::normal_expr::Normalize;
use typecheck::solver::{ConstraintEnv, Predicate, ProveResult};

mod support;

#[test]
fn anonymous_parameter_widening_is_proven_but_narrowing_is_rejected() {
    for (source, target, accepted) in [
        ("0 to 255", "0 to 65535", true),
        ("0 to 255", "-32768 to 32767", true),
        ("-128 to 127", "-32768 to 32767", true),
        ("0 to 65535", "0 to 255", false),
        ("-128 to 127", "0 to 65535", false),
    ] {
        let typed = support::check(&format!(
            "identity(input is in {target}) is in {target}: return input\nconvert(value is in {source}) is in {target}: return identity(value)\n"
        ));
        if accepted {
            assert!(
                typed.all_flaws().is_empty(),
                "{source} -> {target}: {:?}",
                typed.all_flaws()
            );
        } else {
            assert!(
                typed
                    .all_flaws()
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug().starts_with("type-parameter-refinement-")),
                "{source} -> {target}: {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn integer_bounds_prove_lossless_parameter_widening() {
    let value = NormalExpr::Var(internment::Intern::new("value".to_string()));
    let mut constraints = ConstraintEnv::default();
    constraints.assume(&Predicate::Ge(value.clone(), 0_i128.into()));
    constraints.assume(&Predicate::Le(value.clone(), 255_i128.into()));
    for predicate in [
        Predicate::Ge(value.clone(), (-32768_i128).into()),
        Predicate::Le(value.clone(), 65535_i128.into()),
        Predicate::Lt(value.clone(), I256::from(u128::MAX).into()),
    ] {
        assert_eq!(
            constraints.prove(&predicate, &HashMap::new()),
            ProveResult::Proven
        );
    }
    for predicate in [
        Predicate::Lt(value.clone(), 255_i128.into()),
        Predicate::Gt(value.clone(), 0_i128.into()),
        Predicate::Le(value.clone(), 254_i128.into()),
        Predicate::Ge(value.clone(), 1_i128.into()),
    ] {
        assert_eq!(
            constraints.prove(&predicate, &HashMap::new()),
            ProveResult::Unknown
        );
    }
}

#[test]
fn non_strict_refinement_chains_preserve_strictness_and_terminate() {
    let first = NormalExpr::Var(internment::Intern::new("first".to_string()));
    let second = NormalExpr::Var(internment::Intern::new("second".to_string()));
    let third = NormalExpr::Var(internment::Intern::new("third".to_string()));
    let mut constraints = ConstraintEnv::default();
    constraints.assume(&Predicate::Le(first.clone(), second.clone()));
    constraints.assume(&Predicate::Le(second.clone(), third.clone()));
    constraints.assume(&Predicate::Le(third.clone(), first.clone()));
    assert_eq!(
        constraints.prove(
            &Predicate::Le(first.clone(), third.clone()),
            &HashMap::new()
        ),
        ProveResult::Proven
    );
    assert_eq!(
        constraints.prove(
            &Predicate::Lt(first.clone(), third.clone()),
            &HashMap::new()
        ),
        ProveResult::Disproven
    );
    let mut constraints = ConstraintEnv::default();
    constraints.assume(&Predicate::Le(first.clone(), second.clone()));
    constraints.assume(&Predicate::Le(second.clone(), third.clone()));
    assert_eq!(
        constraints.prove(
            &Predicate::Lt(first.clone(), third.clone()),
            &HashMap::new()
        ),
        ProveResult::Unknown
    );
    constraints.assume(&Predicate::Lt(second, third.clone()));
    assert_eq!(
        constraints.prove(&Predicate::Lt(first, third), &HashMap::new()),
        ProveResult::Proven
    );
}

#[test]
fn refinement_comparisons_preserve_wide_integer_values() {
    let constraints = ConstraintEnv::default();
    let wide = I256::from(u128::MAX);
    for (left, right, expected) in [
        (I256::from(0), wide, ProveResult::Proven),
        (wide, I256::from(0), ProveResult::Disproven),
        (wide - I256::from(1), wide, ProveResult::Proven),
        (wide, wide, ProveResult::Disproven),
    ] {
        assert_eq!(
            constraints.prove(&Predicate::Lt(left.into(), right.into()), &HashMap::new()),
            expected
        );
    }
}

#[test]
fn non_integer_constants_do_not_disprove_integer_relations() {
    let value = NormalExpr::Value(ConstValue::Text("unknown".into()));
    assert_eq!(
        ConstraintEnv::default().prove(&Predicate::Lt(value, 1_i128.into()), &HashMap::new()),
        ProveResult::Unknown
    );
}

#[test]
fn refinement_arithmetic_preserves_wide_results_without_wrapping() {
    let wide = I256::from(i128::MAX) + I256::from(1);
    for (expression, expected) in [
        (
            NormalExpr::Add(Box::new(i128::MAX.into()), Box::new(1_i128.into())),
            wide,
        ),
        (
            NormalExpr::Sub(Box::new(0_i128.into()), Box::new(wide.into())),
            I256::from(i128::MIN),
        ),
        (
            NormalExpr::Mul(Box::new(wide.into()), Box::new(2_i128.into())),
            wide * I256::from(2),
        ),
    ] {
        assert_eq!(expression.normalize(), NormalExpr::from(expected));
    }
    let overflow = NormalExpr::Add(Box::new(I256::MAX.into()), Box::new(1_i128.into()));
    assert_eq!(overflow.normalize(), overflow);
}

#[test]
fn partial_checked_arithmetic_does_not_prove_or_disprove_equality() {
    let overflow = NormalExpr::Add(Box::new(I256::MAX.into()), Box::new(1_i128.into()));
    let query = NormalExpr::TargetQuery {
        kind: ast::TargetQueryKind::Size,
        operand: ast::TypeReference::Nominal(internment::Intern::from_ref("Element")),
    };
    for expression in [
        NormalExpr::Mul(Box::new(overflow.clone()), Box::new(0_i128.into())),
        NormalExpr::Sub(Box::new(overflow.clone()), Box::new(overflow)),
        NormalExpr::Mul(Box::new(query), Box::new(0_i128.into())),
    ] {
        for predicate in [
            Predicate::Eq(expression.clone(), 0_i128.into()),
            Predicate::Eq(expression.clone(), 1_i128.into()),
            Predicate::Ne(expression.clone(), 0_i128.into()),
        ] {
            assert_eq!(
                ConstraintEnv::default().prove(&predicate, &HashMap::new()),
                ProveResult::Unknown,
                "{predicate:?}"
            );
        }
    }
}
