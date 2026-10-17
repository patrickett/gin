use ast::{BinderId, BinderOwner, ConstValue, DependentArgId, NormalExpr, TargetQueryKind};
use i256::I256;
use internment::Intern;
use typecheck::{DepSubst, Normalize};

fn parameter(slot: u32) -> NormalExpr {
    NormalExpr::Inferred(DependentArgId::new(
        BinderId::new(0, BinderOwner::Expression(0)),
        slot,
    ))
}

fn add(left: NormalExpr, right: NormalExpr) -> NormalExpr {
    NormalExpr::Add(Box::new(left), Box::new(right))
}

fn subtract(left: NormalExpr, right: NormalExpr) -> NormalExpr {
    NormalExpr::Sub(Box::new(left), Box::new(right))
}

fn multiply(left: NormalExpr, right: NormalExpr) -> NormalExpr {
    NormalExpr::Mul(Box::new(left), Box::new(right))
}

fn target_query() -> NormalExpr {
    NormalExpr::TargetQuery {
        kind: TargetQueryKind::Size,
        operand: ast::TypeReference::Nominal(Intern::from_ref("Element")),
    }
}

#[test]
fn nested_addition_reaches_a_fixed_point_in_one_pass() {
    let expression = add(1_i128.into(), add(2_i128.into(), parameter(0)));
    let normalized = expression.normalize();
    assert_eq!(normalized, add(3_i128.into(), parameter(0)));
    assert_eq!(normalized.normalize(), normalized);
}

#[test]
fn reassociation_preserves_checked_overflow() {
    let expression = add(i256::I256::MAX.into(), add(1_i128.into(), (-1_i128).into()));
    assert_eq!(expression.normalize(), NormalExpr::from(i256::I256::MAX));
    let overflow = add(i256::I256::MAX.into(), 1_i128.into());
    let expression = add(overflow, (-1_i128).into());
    assert_eq!(expression.normalize(), expression);
    assert_eq!(expression.normalize().normalize(), expression);
}

#[test]
fn same_direction_offsets_merge_without_changing_symbol_order() {
    for (first, second) in [(1_i128, 2_i128), (-1, -2)] {
        let sum = NormalExpr::from(first + second);
        for (inner, expected) in [
            (
                add(second.into(), parameter(0)),
                add(sum.clone(), parameter(0)),
            ),
            (add(parameter(0), second.into()), add(parameter(0), sum)),
        ] {
            for expression in [add(first.into(), inner.clone()), add(inner, first.into())] {
                assert_eq!(expression.normalize(), expected);
                assert_eq!(expected.normalize(), expected);
            }
        }
    }
}

#[test]
fn opposite_direction_offsets_keep_their_evaluation_grouping() {
    for (first, second) in [(1_i128, -1_i128), (-1, 1)] {
        for inner in [
            add(second.into(), parameter(0)),
            add(parameter(0), second.into()),
        ] {
            for expression in [add(first.into(), inner.clone()), add(inner, first.into())] {
                assert_eq!(expression.normalize(), expression);
            }
        }
    }
}

#[test]
fn checked_folding_preserves_wide_values_and_all_overflow_operations() {
    let wide = I256::from(i128::MAX).checked_add(I256::from(1)).unwrap();
    for (expression, expected) in [
        (add(i128::MAX.into(), 1_i128.into()), wide),
        (subtract(0_i128.into(), wide.into()), I256::from(i128::MIN)),
        (
            multiply(wide.into(), 2_i128.into()),
            wide.checked_mul(I256::from(2)).unwrap(),
        ),
    ] {
        assert_eq!(expression.normalize(), NormalExpr::from(expected));
        assert_eq!(
            expression.normalize_to_value(),
            Some(ConstValue::Int(expected))
        );
    }
    for expression in [
        add(I256::MAX.into(), 1_i128.into()),
        subtract(I256::MIN.into(), 1_i128.into()),
        multiply(I256::MAX.into(), 2_i128.into()),
    ] {
        assert_eq!(expression.normalize(), expression);
        assert_eq!(expression.normalize_to_value(), None);
    }
}

#[test]
fn integer_atom_identities_preserve_binders() {
    let value = parameter(0);
    for expression in [
        add(value.clone(), 0_i128.into()),
        add(0_i128.into(), value.clone()),
        subtract(value.clone(), 0_i128.into()),
        multiply(value.clone(), 1_i128.into()),
        multiply(1_i128.into(), value.clone()),
    ] {
        assert_eq!(expression.normalize(), value);
    }
    for expression in [
        subtract(value.clone(), value.clone()),
        multiply(value.clone(), 0_i128.into()),
        multiply(0_i128.into(), value),
    ] {
        assert_eq!(expression.normalize(), NormalExpr::from(0));
    }
    let different = subtract(parameter(0), parameter(1));
    assert_eq!(different.normalize(), different);
}

#[test]
fn cancellation_does_not_erase_partial_integer_evaluation() {
    for partial in [
        add(I256::MAX.into(), 1_i128.into()),
        add(parameter(0), 1_i128.into()),
        target_query(),
    ] {
        for expression in [
            multiply(partial.clone(), 0_i128.into()),
            multiply(0_i128.into(), partial.clone()),
            subtract(partial.clone(), partial.clone()),
        ] {
            assert_eq!(expression.normalize(), expression);
            assert_eq!(expression.normalize_to_value(), None);
        }
    }
}

#[test]
fn unsupported_values_are_not_subject_to_integer_identities() {
    for value in [
        ConstValue::Text("text".into()),
        ConstValue::Float(ast::HashFloat(1.5)),
    ] {
        let value = NormalExpr::Value(value);
        for expression in [
            add(value.clone(), 0_i128.into()),
            subtract(value.clone(), value.clone()),
            multiply(value.clone(), 0_i128.into()),
            multiply(value.clone(), 1_i128.into()),
        ] {
            assert_eq!(expression.normalize(), expression);
        }
    }
}

#[test]
fn target_queries_remain_symbolic_until_substitution_and_materialization() {
    let query = target_query();
    assert_eq!(add(query.clone(), 0_i128.into()).normalize(), query);
    assert_eq!(multiply(query.clone(), 1_i128.into()).normalize(), query);
    let mut substitution = DepSubst::new();
    substitution
        .types
        .insert(Intern::from_ref("Element"), ast::Ty::Unit);
    assert_eq!(
        substitution.apply_to_normal(&query).normalize(),
        NormalExpr::TargetQuery {
            kind: TargetQueryKind::Size,
            operand: ast::TypeReference::Unit
        }
    );
}

#[test]
fn dependent_substitution_precedes_checked_normalization() {
    let name = Intern::from_ref("n");
    let expression = add(1_i128.into(), add(2_i128.into(), NormalExpr::Var(name)));
    for value in [I256::MIN, I256::from(4), I256::MAX] {
        let mut substitution = DepSubst::new();
        substitution.consts.insert(name, value.into());
        let substituted = substitution.apply_to_normal(&expression);
        let normalized = substituted.normalize();
        assert_eq!(normalized.normalize(), normalized);
        assert_eq!(
            checked_value(&normalized, value),
            checked_value(&substituted, value)
        );
    }
    let original = match parameter(0) {
        NormalExpr::Inferred(id) => id,
        _ => unreachable!(),
    };
    let target = BinderId::new(1, BinderOwner::Expression(0));
    let mut instantiation = typecheck::subst::DependentInstantiation::new(original.binder, target);
    let renamed = instantiation.apply_to_normal(&add(parameter(0), 0_i128.into()));
    assert_eq!(
        renamed.normalize(),
        NormalExpr::Inferred(DependentArgId::new(target, 0))
    );
}

fn checked_value(expression: &NormalExpr, binding: I256) -> Option<I256> {
    let mut substitution = DepSubst::new();
    substitution
        .consts
        .insert(Intern::from_ref("n"), binding.into());
    let expression = substitution.apply_to_normal(expression);
    let domain = ast::integer::IntegerDomain::from_refinement(&ast::PredicateExpr::Eq(expression));
    let hull = domain.storage_hull()?;
    assert_eq!(hull.min(), hull.max());
    Some(hull.min())
}

fn arithmetic_atoms() -> Vec<NormalExpr> {
    let mut atoms: Vec<_> = [
        I256::MIN,
        I256::MAX,
        I256::from(-1),
        I256::from(0),
        I256::from(1),
        I256::from(2),
    ]
    .into_iter()
    .map(NormalExpr::from)
    .collect();
    atoms.extend([
        NormalExpr::Var(Intern::from_ref("n")),
        target_query(),
        NormalExpr::Value(ConstValue::Text("unsupported".into())),
    ]);
    atoms
}

#[test]
fn bounded_arithmetic_corpus_is_idempotent_and_preserves_checked_evaluation() {
    let atoms = arithmetic_atoms();
    let operations: [fn(NormalExpr, NormalExpr) -> NormalExpr; 3] = [add, subtract, multiply];
    let mut operands = atoms.clone();
    for left in &atoms {
        for right in &atoms {
            for operation in operations {
                operands.push(operation(left.clone(), right.clone()));
            }
        }
    }
    for operand in &operands {
        for atom in &atoms {
            for operation in operations {
                for expression in [
                    operation(operand.clone(), atom.clone()),
                    operation(atom.clone(), operand.clone()),
                ] {
                    let normalized = expression.normalize();
                    assert_eq!(normalized.normalize(), normalized, "{expression:?}");
                    for binding in [
                        I256::MIN,
                        I256::MIN + I256::from(2),
                        I256::from(-1),
                        I256::from(0),
                        I256::from(1),
                        I256::MAX - I256::from(2),
                        I256::MAX,
                    ] {
                        assert_eq!(
                            checked_value(&normalized, binding),
                            checked_value(&expression, binding),
                            "{expression:?} with {binding}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn deeply_nested_identities_use_an_explicit_work_stack() {
    let mut expression = parameter(0);
    for _ in 0..8192 {
        expression = add(expression, 0_i128.into());
    }
    assert_eq!(expression.normalize(), parameter(0));
    for _ in 0..8192 {
        let NormalExpr::Add(left, right) = expression else {
            panic!("nested addition")
        };
        assert_eq!(*right, NormalExpr::from(0));
        expression = *left;
    }
    assert_eq!(expression, parameter(0));
}
