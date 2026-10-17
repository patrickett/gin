use ast::{BinderId, BinderOwner, DependentArgId, NormalExpr};
use std::collections::HashMap;
use typecheck::{
    ConstraintEnv, Overlap, PlaceId, Predicate, ProveResult, ReferenceTargetGroup, TargetIndex,
};

fn inferred(slot: u32) -> NormalExpr {
    NormalExpr::Inferred(DependentArgId::new(
        BinderId::new(0, BinderOwner::Expression(0)),
        slot,
    ))
}

#[test]
fn linear_capacity_bounds_combine_counts_and_element_sizes() {
    let length = inferred(0);
    let capacity = inferred(1);
    let bytes = |count: NormalExpr| NormalExpr::Mul(Box::new(count), Box::new(8_i128.into()));
    let mut env = ConstraintEnv::default();
    env.assume(&Predicate::Le(length.clone(), capacity.clone()));
    env.assume(&Predicate::Le(bytes(capacity), 120_i128.into()));
    assert_eq!(env.prove(&Predicate::Le(bytes(length.clone()), 120_i128.into()), &HashMap::new()), ProveResult::Proven);
    assert_ne!(env.prove(&Predicate::Le(bytes(length), 119_i128.into()), &HashMap::new()), ProveResult::Proven);
}

#[test]
fn subtraction_guards_prove_addition_preconditions_without_runtime_addition() {
    let length = inferred(0);
    let additional = inferred(1);
    let mut env = ConstraintEnv::default();
    env.assume(&Predicate::Le(additional.clone(), NormalExpr::Sub(Box::new(127_i128.into()), Box::new(length.clone()))));
    let sum = NormalExpr::Add(Box::new(length), Box::new(additional));
    assert_eq!(env.prove(&Predicate::Le(sum.clone(), 127_i128.into()), &HashMap::new()), ProveResult::Proven);
    assert_ne!(env.prove(&Predicate::Lt(sum, 127_i128.into()), &HashMap::new()), ProveResult::Proven);
}

#[test]
fn linear_scaling_preserves_integer_strictness_and_rejects_nonlinear_assumptions() {
    let value = inferred(0);
    let mut env = ConstraintEnv::default();
    env.assume(&Predicate::Lt(value.clone(), 8_i128.into()));
    let doubled = NormalExpr::Mul(Box::new(value.clone()), Box::new(2_i128.into()));
    assert_eq!(env.prove(&Predicate::Le(doubled.clone(), 14_i128.into()), &HashMap::new()), ProveResult::Proven);
    assert_ne!(env.prove(&Predicate::Lt(doubled, 14_i128.into()), &HashMap::new()), ProveResult::Proven);
    let product = NormalExpr::Mul(Box::new(value), Box::new(inferred(1)));
    assert_eq!(env.prove(&Predicate::Le(product, 64_i128.into()), &HashMap::new()), ProveResult::Unknown);
}

#[test]
fn distinct_inferred_values_do_not_prove_inequality() {
    let env = ConstraintEnv::default();
    for predicate in [
        Predicate::Eq(inferred(0), inferred(1)),
        Predicate::Ne(inferred(0), inferred(1)),
    ] {
        assert_eq!(env.prove(&predicate, &HashMap::new()), ProveResult::Unknown);
    }
}

#[test]
fn assumed_equality_between_inferred_values_is_respected() {
    let predicate = Predicate::Eq(inferred(0), inferred(1));
    let mut env = ConstraintEnv::default();
    env.assume(&predicate);
    assert_eq!(env.prove(&predicate, &HashMap::new()), ProveResult::Proven);
}

#[test]
fn distinct_inferred_indices_are_not_proven_disjoint() {
    let item = |slot| ReferenceTargetGroup::ItemRegion {
        base: Box::new(ReferenceTargetGroup::Local(PlaceId(0))),
        index: TargetIndex::Symbolic(inferred(slot)),
    };
    assert_eq!(
        item(0).overlap(&item(1), &ConstraintEnv::default()),
        Overlap::Unknown
    );
}

#[test]
fn inferred_indices_can_be_proven_disjoint_by_ordering() {
    let mut env = ConstraintEnv::default();
    env.assume(&Predicate::Lt(inferred(0), inferred(1)));
    let item = |slot| ReferenceTargetGroup::ItemRegion {
        base: Box::new(ReferenceTargetGroup::Local(PlaceId(0))),
        index: TargetIndex::Symbolic(inferred(slot)),
    };
    assert_eq!(item(0).overlap(&item(1), &env), Overlap::Disjoint);
}

#[test]
fn a_known_field_value_disproves_an_incompatible_bound() {
    let field = NormalExpr::Var(internment::Intern::from_ref("region.$field1"));
    let mut env = ConstraintEnv::default();
    env.assume(&Predicate::Eq(field.clone(), 2_i128.into()));
    for predicate in [
        Predicate::Lt(field.clone(), 2_i128.into()),
        Predicate::Gt(field.clone(), 2_i128.into()),
        Predicate::Ge(field, 3_i128.into()),
    ] {
        assert_eq!(
            env.prove(&predicate, &HashMap::new()),
            ProveResult::Disproven
        );
    }
}

#[test]
fn substitutions_apply_inside_arithmetic_constraint_operands() {
    let count = internment::Intern::from_ref("self.count");
    let expression = ast::PredicateExpr::Lt(NormalExpr::Add(
        Box::new(NormalExpr::Var(count)),
        Box::new(1_i128.into()),
    ));
    let predicate = typecheck::solver::predicate_expr_to_predicate(
        &expression,
        2_i128.into(),
        &HashMap::from([(count, 2_i128.into())]),
    );
    assert_eq!(
        ConstraintEnv::default().prove(&predicate, &HashMap::new()),
        ProveResult::Proven
    );
}

#[test]
fn a_nonnegative_count_that_is_not_positive_is_zero() {
    let count = NormalExpr::Var(internment::Intern::from_ref("self.$field1"));
    let mut env = ConstraintEnv::default();
    env.assume(&Predicate::Ge(count.clone(), 0_i128.into()));
    env.assume(&Predicate::Le(count.clone(), 0_i128.into()));
    assert_eq!(
        env.prove(&Predicate::Eq(count, 0_i128.into()), &HashMap::new()),
        ProveResult::Proven
    );
}

#[test]
fn a_strict_bound_disproves_equality() {
    let count = NormalExpr::Var(internment::Intern::from_ref("self.$field1"));
    let mut env = ConstraintEnv::default();
    env.assume(&Predicate::Gt(count.clone(), 0_i128.into()));
    assert_eq!(
        env.prove(&Predicate::Eq(count, 0_i128.into()), &HashMap::new()),
        ProveResult::Disproven
    );
}

#[test]
fn checked_doubling_preserves_symbolic_byte_extent_bounds() {
    let count = inferred(0);
    let width = inferred(1);
    let product = NormalExpr::Mul(Box::new(count.clone()), Box::new(width.clone()));
    let mut env = ConstraintEnv::default();
    env.assume(&Predicate::Le(product.clone(), NormalExpr::Sub(Box::new(128_i128.into()), Box::new(product))));
    let doubled = NormalExpr::Mul(Box::new(count.clone()), Box::new(2_i128.into()));
    let doubled_bytes = NormalExpr::Mul(Box::new(doubled), Box::new(width));
    assert_eq!(env.prove(&Predicate::Le(doubled_bytes.clone(), 128_i128.into()), &HashMap::new()), ProveResult::Proven);
    assert_ne!(env.prove(&Predicate::Lt(doubled_bytes, 128_i128.into()), &HashMap::new()), ProveResult::Proven);
    assert_ne!(env.prove(&Predicate::Le(count, 64_i128.into()), &HashMap::new()), ProveResult::Proven);
}
