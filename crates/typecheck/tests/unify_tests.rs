use ast::{Ty, TyArg};
use internment::Intern;
use typecheck::analysis::unify_type_args;

#[test]
fn constant_unification_normalizes_identity_expressions() {
    let parameter = ast::NormalExpr::Inferred(ast::DependentArgId::new(
        ast::BinderId::new(0, ast::BinderOwner::Expression(0)),
        0,
    ));
    let identity = ast::NormalExpr::Add(Box::new(parameter.clone()), Box::new(0_i128.into()));
    for (expected, actual) in [(identity.clone(), parameter.clone()), (parameter, identity)] {
        let subst = unify_type_args(&[TyArg::Const(expected)], &[TyArg::Const(actual)])
            .expect("normalized identity");
        assert!(subst.consts.is_empty());
        assert!(subst.types.is_empty());
    }
}

fn fixed_parameter(file: u32, slot: u32) -> ast::NormalExpr {
    ast::NormalExpr::Inferred(ast::DependentArgId::new(
        ast::BinderId::new(file, ast::BinderOwner::Expression(0)),
        slot,
    ))
}

#[test]
fn normalized_constant_patterns_preserve_inference_and_conflict_diagnostics() {
    use ast::NormalExpr;

    let name = Intern::from_ref("count");
    let pattern = NormalExpr::Add(Box::new(NormalExpr::Var(name)), Box::new(0_i128.into()));
    let actual = NormalExpr::Add(Box::new(1_i128.into()), Box::new(2_i128.into()));
    let subst = unify_type_args(
        &[
            TyArg::Const(pattern.clone()),
            TyArg::Const(NormalExpr::Var(name)),
        ],
        &[TyArg::Const(actual.clone()), TyArg::Const(3_i128.into())],
    )
    .expect("normalized repeated binding");
    assert_eq!(subst.consts.get(&name), Some(&NormalExpr::from(3)));
    let error = unify_type_args(
        &[TyArg::Const(pattern), TyArg::Const(NormalExpr::Var(name))],
        &[TyArg::Const(actual), TyArg::Const(4_i128.into())],
    )
    .err()
    .expect("conflicting repeated binding");
    assert_eq!(error, "const parameter `count` inferred as both 3 and 4");
}

#[test]
fn arithmetic_structure_can_infer_parameters_before_constant_folding() {
    use ast::NormalExpr;

    let name = Intern::from_ref("count");
    let variable = NormalExpr::Var(name);
    let operations: [fn(Box<NormalExpr>, Box<NormalExpr>) -> NormalExpr; 3] =
        [NormalExpr::Add, NormalExpr::Sub, NormalExpr::Mul];
    for operation in operations {
        let expected = operation(Box::new(variable.clone()), Box::new(variable.clone()));
        let actual = operation(Box::new(1_i128.into()), Box::new(1_i128.into()));
        let subst = unify_type_args(&[TyArg::Const(expected)], &[TyArg::Const(actual)])
            .expect("structural parameter inference");
        assert_eq!(subst.consts.get(&name), Some(&NormalExpr::from(1)));
    }
}

#[test]
fn failed_structural_attempt_does_not_leak_parameter_bindings() {
    use ast::NormalExpr;

    let name = Intern::from_ref("count");
    let parameter = fixed_parameter(0, 0);
    let expected = NormalExpr::Add(Box::new(NormalExpr::Var(name)), Box::new(0_i128.into()));
    let actual = NormalExpr::Add(Box::new(0_i128.into()), Box::new(parameter.clone()));
    let subst = unify_type_args(&[TyArg::Const(expected)], &[TyArg::Const(actual)])
        .expect("normalized retry");
    assert_eq!(subst.consts.get(&name), Some(&parameter));
}

#[test]
fn normalized_symbolic_bindings_keep_dependent_binder_identity() {
    use ast::NormalExpr;

    let name = Intern::from_ref("count");
    let parameter = fixed_parameter(0, 0);
    let identity = NormalExpr::Mul(Box::new(parameter.clone()), Box::new(1_i128.into()));
    let expected = [
        TyArg::Const(NormalExpr::Var(name)),
        TyArg::Const(NormalExpr::Var(name)),
    ];
    let subst = unify_type_args(
        &expected,
        &[TyArg::Const(identity), TyArg::Const(parameter.clone())],
    )
    .expect("same normalized symbolic binding");
    assert_eq!(subst.consts.get(&name), Some(&parameter));
    for different in [fixed_parameter(1, 0), fixed_parameter(0, 1)] {
        assert!(
            unify_type_args(
                &[TyArg::Const(parameter.clone())],
                &[TyArg::Const(different.clone())]
            )
            .is_err()
        );
        let error = unify_type_args(
            &expected,
            &[TyArg::Const(parameter.clone()), TyArg::Const(different)],
        )
        .err()
        .expect("distinct dependent parameters");
        assert!(error.contains("inferred as both"));
    }
}

#[test]
fn nested_arrays_and_named_arguments_share_constant_normalization() {
    use ast::NormalExpr;

    let parameter = fixed_parameter(0, 0);
    let identity = NormalExpr::Add(Box::new(parameter.clone()), Box::new(0_i128.into()));
    let array = |size| Ty::Ref {
        mutable: false,
        inner: Box::new(Ty::Array {
            elem: Box::new(Ty::Unit),
            size,
        }),
    };
    assert!(
        unify_type_args(
            &[record(&[("bytes", array(identity.clone()))])],
            &[record(&[("bytes", array(parameter.clone()))])],
        )
        .is_ok()
    );
    let name = Intern::from_ref("Sized");
    let argument = Intern::from_ref("length");
    let named = |size| {
        TyArg::Type(Box::new(Ty::Named {
            name,
            instance: ast::ty::NamedTypeInstance::new(ast::ty::TypeId { file: 0, name })
                .with_arguments(vec![(argument, TyArg::Const(size))]),
        }))
    };
    assert!(unify_type_args(&[named(identity.clone())], &[named(parameter.clone())]).is_ok());
    for is_union in [false, true] {
        assert!(
            unify_type_args(
                &[structural_parameters(
                    is_union,
                    vec![(argument, TyArg::Const(identity.clone()))]
                )],
                &[structural_parameters(
                    is_union,
                    vec![(argument, TyArg::Const(parameter.clone()))]
                )],
            )
            .is_ok()
        );
    }
}

#[test]
fn repeated_type_parameters_compare_normalized_dependent_indices() {
    use ast::NormalExpr;

    let name = Intern::from_ref("T");
    let parameter = fixed_parameter(0, 0);
    let identity = NormalExpr::Add(Box::new(parameter.clone()), Box::new(0_i128.into()));
    let array = |size| Ty::Array {
        elem: Box::new(Ty::Unit),
        size,
    };
    let pattern = TyArg::Type(Box::new(Ty::Opaque(name)));
    let expected = [pattern.clone(), pattern];
    let subst = unify_type_args(
        &expected,
        &[
            TyArg::Type(Box::new(array(identity))),
            TyArg::Type(Box::new(array(parameter.clone()))),
        ],
    )
    .expect("same dependent array type");
    assert_eq!(subst.types.get(&name), Some(&array(parameter)));
    let error = unify_type_args(
        &expected,
        &[
            TyArg::Type(Box::new(array(fixed_parameter(0, 0)))),
            TyArg::Type(Box::new(array(fixed_parameter(1, 0)))),
        ],
    )
    .err()
    .expect("different dependent array type");
    assert!(error.contains("type parameter `T` inferred as both"));
}

#[test]
fn matching_does_not_wrap_overflow_or_solve_arithmetic_equations() {
    use ast::NormalExpr;

    let overflow = NormalExpr::Add(Box::new(i256::I256::MAX.into()), Box::new(1_i128.into()));
    assert!(
        unify_type_args(
            &[TyArg::Const(overflow)],
            &[TyArg::Const(i256::I256::MIN.into())]
        )
        .is_err()
    );
    let name = Intern::from_ref("count");
    let pattern = NormalExpr::Add(Box::new(NormalExpr::Var(name)), Box::new(1_i128.into()));
    assert!(unify_type_args(&[TyArg::Const(pattern)], &[TyArg::Const(4_i128.into())]).is_err());
}

#[test]
fn matching_does_not_import_refinement_assumptions() {
    let left = fixed_parameter(0, 0);
    let right = fixed_parameter(0, 1);
    let mut constraints = typecheck::ConstraintEnv::default();
    let predicate = typecheck::Predicate::Eq(left.clone(), right.clone());
    assert_eq!(
        constraints.prove(&predicate, &std::collections::HashMap::new()),
        typecheck::ProveResult::Unknown
    );
    constraints.assume(&predicate);
    assert_eq!(
        constraints.prove(&predicate, &std::collections::HashMap::new()),
        typecheck::ProveResult::Proven
    );
    assert!(unify_type_args(&[TyArg::Const(left)], &[TyArg::Const(right)]).is_err());
}

fn record(fields: &[(&str, Ty)]) -> TyArg {
    TyArg::Type(Box::new(Ty::Record {
        name: Intern::from_ref("Record"),
        fields: fields
            .iter()
            .map(|(name, ty)| (Intern::from_ref(*name), Box::new(ty.clone())))
            .collect(),
        resolved_params: None,
    }))
}

#[test]
fn record_unification_rejects_different_field_counts() {
    let expected = record(&[("first", Ty::Unit)]);
    let actual = record(&[]);
    assert!(
        unify_type_args(
            std::slice::from_ref(&expected),
            std::slice::from_ref(&actual)
        )
        .is_err()
    );
    assert!(unify_type_args(&[actual], &[expected]).is_err());
}

#[test]
fn record_unification_rejects_different_field_names() {
    assert!(
        unify_type_args(
            &[record(&[("first", Ty::Unit)])],
            &[record(&[("second", Ty::Unit)])]
        )
        .is_err()
    );
}

#[test]
fn record_unification_rejects_reordered_fields() {
    assert!(
        unify_type_args(
            &[record(&[("first", Ty::Unit), ("second", Ty::Unit)])],
            &[record(&[("second", Ty::Unit), ("first", Ty::Unit)])]
        )
        .is_err()
    );
}

#[test]
fn record_unification_preserves_generic_field_inference() {
    let name = Intern::from_ref("T");
    let subst = unify_type_args(
        &[record(&[("field", Ty::Opaque(name))])],
        &[record(&[("field", Ty::Unit)])],
    )
    .expect("matching record shape");
    assert_eq!(subst.types.get(&name), Some(&Ty::Unit));
}

fn union(variants: Vec<ast::ty::UnionVariant>) -> TyArg {
    TyArg::Type(Box::new(Ty::union_named(
        Intern::from_ref("(Some or None)"),
        variants,
    )))
}

fn structural_parameters(is_union: bool, parameters: Vec<(Intern<String>, TyArg)>) -> TyArg {
    let name = Intern::from_ref("Carrier");
    TyArg::Type(Box::new(if is_union {
        Ty::Union {
            name,
            variants: Vec::new(),
            literal_values: None,
            resolved_params: Some(parameters),
        }
    } else {
        Ty::Record {
            name,
            fields: Vec::new(),
            resolved_params: Some(parameters),
        }
    }))
}

#[test]
fn structural_parameter_metadata_preserves_unused_parameter_inference() {
    let element = Intern::from_ref("element");
    let length = Intern::from_ref("length");
    let t = Intern::from_ref("T");
    let n = Intern::from_ref("N");
    for is_union in [false, true] {
        let expected = structural_parameters(
            is_union,
            vec![
                (element, TyArg::Type(Box::new(Ty::Opaque(t)))),
                (length, TyArg::Const(ast::NormalExpr::Var(n))),
            ],
        );
        let actual = structural_parameters(
            is_union,
            vec![
                (element, TyArg::Type(Box::new(Ty::Unit))),
                (length, TyArg::Const(4_i128.into())),
            ],
        );
        let subst = unify_type_args(
            std::slice::from_ref(&expected),
            std::slice::from_ref(&actual),
        )
        .expect("matching structural parameters");
        assert_eq!(subst.types.get(&t), Some(&Ty::Unit));
        assert_eq!(subst.consts.get(&n), Some(&4_i128.into()));
        let TyArg::Type(expected) = expected else {
            unreachable!()
        };
        let TyArg::Type(actual) = actual else {
            unreachable!()
        };
        assert_eq!(subst.apply_to_ty(&expected), *actual);
    }
}

#[test]
fn structural_parameter_metadata_rejects_incompatible_arguments() {
    let element = Intern::from_ref("element");
    let length = Intern::from_ref("length");
    let unit = (element, TyArg::Type(Box::new(Ty::Unit)));
    let size = (length, TyArg::Const(3_i128.into()));
    for is_union in [false, true] {
        let expected = structural_parameters(is_union, vec![unit.clone(), size.clone()]);
        for parameters in [
            vec![unit.clone()],
            vec![size.clone(), unit.clone()],
            vec![(Intern::from_ref("other"), unit.1.clone()), size.clone()],
            vec![(element, TyArg::Const(3_i128.into())), size.clone()],
            vec![unit.clone(), (length, TyArg::Const(4_i128.into()))],
            vec![
                (
                    element,
                    TyArg::Type(Box::new(Ty::Ptr {
                        inner: Box::new(Ty::Unit),
                    })),
                ),
                size.clone(),
            ],
        ] {
            let actual = structural_parameters(is_union, parameters);
            assert!(
                unify_type_args(
                    std::slice::from_ref(&expected),
                    std::slice::from_ref(&actual)
                )
                .is_err()
            );
            assert!(unify_type_args(&[actual], std::slice::from_ref(&expected)).is_err());
        }
    }
}

#[test]
fn structural_parameter_metadata_agrees_with_payload_inference() {
    let element = Intern::from_ref("element");
    let parameter = Intern::from_ref("T");
    for is_union in [false, true] {
        let with_payload = |argument: TyArg, payload: Ty| {
            let TyArg::Type(mut ty) = structural_parameters(is_union, vec![(element, argument)])
            else {
                unreachable!()
            };
            let fields = vec![(Intern::from_ref("value"), Box::new(payload))];
            match ty.as_mut() {
                Ty::Record { fields: target, .. } => *target = fields,
                Ty::Union { variants, .. } => {
                    *variants = vec![ast::ty::UnionVariant::new(Intern::from_ref("Some"), fields)];
                }
                _ => unreachable!(),
            }
            TyArg::Type(ty)
        };
        let expected = with_payload(
            TyArg::Type(Box::new(Ty::Opaque(parameter))),
            Ty::Opaque(parameter),
        );
        let actual = with_payload(TyArg::Type(Box::new(Ty::Unit)), Ty::Unit);
        assert!(unify_type_args(std::slice::from_ref(&expected), &[actual]).is_ok());
        let conflicting = with_payload(
            TyArg::Type(Box::new(Ty::Unit)),
            Ty::Ptr {
                inner: Box::new(Ty::Unit),
            },
        );
        assert!(unify_type_args(&[expected], &[conflicting]).is_err());
    }
}

#[test]
fn union_unification_preserves_generic_payload_inference() {
    let parameter = Intern::from_ref("T");
    let variant = |ty| {
        ast::ty::UnionVariant::new(
            Intern::from_ref("Some"),
            vec![(Intern::from_ref("value"), Box::new(ty))],
        )
    };
    let subst = unify_type_args(
        &[union(vec![variant(Ty::Opaque(parameter))])],
        &[union(vec![variant(Ty::Unit)])],
    )
    .expect("matching union shape");
    assert_eq!(subst.types.get(&parameter), Some(&Ty::Unit));
    let mut expected = variant(Ty::Unit);
    expected.result_ty = Some(Ty::Opaque(parameter));
    let mut actual = variant(Ty::Unit);
    actual.result_ty = Some(Ty::Unit);
    let subst = unify_type_args(&[union(vec![expected])], &[union(vec![actual])])
        .expect("matching indexed union shape");
    assert_eq!(subst.types.get(&parameter), Some(&Ty::Unit));
}

#[test]
fn union_unification_rejects_reordered_variants_and_different_literals() {
    let first = ast::ty::UnionVariant::new(Intern::from_ref("First"), Vec::new());
    let second = ast::ty::UnionVariant::new(Intern::from_ref("Second"), Vec::new());
    assert!(
        unify_type_args(
            &[union(vec![first.clone(), second.clone()])],
            &[union(vec![second, first])],
        )
        .is_err()
    );
    let literal = |value| {
        TyArg::Type(Box::new(Ty::union_of_literals(
            Intern::from_ref("Choices"),
            vec![ast::ConstValue::Int(value)],
        )))
    };
    let first = literal(1_i128.into());
    assert!(unify_type_args(std::slice::from_ref(&first), std::slice::from_ref(&first)).is_ok());
    assert!(unify_type_args(&[first], &[literal(2_i128.into())]).is_err());
}

#[test]
fn union_unification_rejects_incompatible_variant_shapes() {
    let variant = ast::ty::UnionVariant::new(
        Intern::from_ref("Some"),
        vec![(Intern::from_ref("value"), Box::new(Ty::Unit))],
    );
    let mut renamed = variant.clone();
    renamed.name = Intern::from_ref("Other");
    let mut kind = variant.clone();
    kind.kind = ast::ty::UnionVariantKind::Type;
    let mut missing_field = variant.clone();
    missing_field.fields.clear();
    let mut field_name = variant.clone();
    field_name.fields[0].0 = Intern::from_ref("other");
    let mut payload = variant.clone();
    *payload.fields[0].1 = Ty::Ptr {
        inner: Box::new(Ty::Unit),
    };
    let mut result = variant.clone();
    result.result_ty = Some(Ty::Unit);
    let expected = union(vec![variant.clone()]);
    for variants in [
        Vec::new(),
        vec![variant.clone(), variant.clone()],
        vec![renamed],
        vec![kind],
        vec![missing_field],
        vec![field_name],
        vec![payload],
        vec![result],
    ] {
        let actual = union(variants);
        assert!(
            unify_type_args(
                std::slice::from_ref(&expected),
                std::slice::from_ref(&actual)
            )
            .is_err()
        );
        assert!(unify_type_args(&[actual], std::slice::from_ref(&expected)).is_err());
    }
}

#[test]
fn constant_unification_accepts_identical_target_queries() {
    use ast::{NormalExpr, TargetQueryKind, TypeReference};

    let operand = TypeReference::Nominal(Intern::from_ref("Element"));
    for kind in [
        TargetQueryKind::Size,
        TargetQueryKind::Alignment,
        TargetQueryKind::IndexBits,
    ] {
        let query = NormalExpr::TargetQuery {
            kind,
            operand: operand.clone(),
        };
        for expression in [
            query.clone(),
            NormalExpr::Add(Box::new(query.clone()), Box::new(1_i128.into())),
        ] {
            let argument = TyArg::Const(expression.clone());
            let subst = unify_type_args(
                std::slice::from_ref(&argument),
                std::slice::from_ref(&argument),
            )
            .expect("identical symbolic lengths must unify");
            assert!(subst.types.is_empty());
            assert!(subst.consts.is_empty());
            let array = TyArg::Type(Box::new(Ty::Array {
                elem: Box::new(Ty::Unit),
                size: expression,
            }));
            assert!(
                unify_type_args(std::slice::from_ref(&array), std::slice::from_ref(&array)).is_ok()
            );
        }
        for different in [
            NormalExpr::TargetQuery {
                kind,
                operand: TypeReference::Unit,
            },
            NormalExpr::TargetQuery {
                kind: match kind {
                    TargetQueryKind::Size => TargetQueryKind::Alignment,
                    _ => TargetQueryKind::Size,
                },
                operand: operand.clone(),
            },
        ] {
            assert!(
                unify_type_args(&[TyArg::Const(query.clone())], &[TyArg::Const(different)])
                    .is_err()
            );
        }
    }
}
