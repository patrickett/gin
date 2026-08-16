use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, FileId};

#[test]
fn explicit_anonymous_result_surface_has_callable_owner() {
    let source = "choose(value) True or False: True";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let ty = &typed.defs[&DefId(Intern::from_ref("choose"))].return_type;
    let ast::Ty::ResultFamily {
        owner: ast::ResultFamilyOwner::Callable(owner),
        alternatives,
    } = ty
    else {
        panic!("expected callable-owned result family, got {ty:?}");
    };

    assert_eq!(owner.declaration.as_str(), "choose");
    assert_eq!(
        alternatives
            .iter()
            .map(|alternative| alternative.label.as_str())
            .collect::<Vec<_>>(),
        ["True", "False"]
    );
    let BindBody::Expr(body) = typed.defs[&DefId(Intern::from_ref("choose"))].body else {
        panic!("expected expression body");
    };
    assert_eq!(
        typed.exprs.const_value[body.as_usize()],
        Some(ast::ConstValue::ResultAlternative {
            owner: ast::ResultFamilyOwner::Callable(owner.clone()),
            label: Intern::from_ref("True"),
        })
    );
    assert_eq!(
        typed.exprs.kind[body.as_usize()],
        typecheck::TypedExprKind::TagCall {
            variant_id: typecheck::VariantId {
                union: typecheck::TagId(Intern::from_ref("True")),
                name: Intern::from_ref("True"),
            },
            discriminant: 0,
            args: Some(Vec::new()),
            field_names: Vec::new(),
        }
    );
}

#[test]
fn inferred_alternative_construction_introduces_callable_family() {
    let typed = transform(
        &TokenCursor::parse_source("choose: Ready"),
        FileId(0),
        &TransformCtx::new(),
    );
    let bind = &typed.defs[&DefId(Intern::from_ref("choose"))];
    let ast::Ty::ResultFamily {
        owner: ast::ResultFamilyOwner::Callable(owner),
        alternatives,
    } = &bind.return_type
    else {
        panic!("expected inferred callable-owned family");
    };

    assert_eq!(owner.declaration.as_str(), "choose");
    assert_eq!(alternatives.len(), 1);
    assert_eq!(alternatives[0].label.as_str(), "Ready");
    let BindBody::Expr(body) = bind.body else {
        panic!("expected expression body");
    };
    assert_eq!(typed.exprs.ty[body.as_usize()], bind.return_type);
    assert!(matches!(
        &typed.exprs.const_value[body.as_usize()],
        Some(ast::ConstValue::ResultAlternative { owner: value_owner, label })
            if value_owner == &ast::ResultFamilyOwner::Callable(owner.clone())
                && label.as_str() == "Ready"
    ));
}

#[test]
fn inferred_branch_exits_extend_one_callable_family() {
    let source = "choose(value):\n\tif value is 0\n\t\tTrue\n\treturn\n\treturn False";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let bind = &typed.defs[&DefId(Intern::from_ref("choose"))];
    let ast::Ty::ResultFamily {
        owner: ast::ResultFamilyOwner::Callable(owner),
        alternatives,
    } = &bind.return_type
    else {
        panic!("expected inferred callable-owned family");
    };

    assert_eq!(owner.declaration.as_str(), "choose");
    assert_eq!(
        alternatives
            .iter()
            .map(|alternative| alternative.label.as_str())
            .collect::<Vec<_>>(),
        ["True", "False"]
    );
    let BindBody::Body {
        exprs,
        ret: Some(false_exit),
    } = &bind.body
    else {
        panic!("expected branching body");
    };
    let typecheck::TypedExprKind::If(if_expr) = &typed.exprs.kind[exprs[0].as_usize()] else {
        panic!("expected if exit");
    };
    let true_exit = *if_expr.stmts.last().expect("taken exit");
    for (exit, label) in [(true_exit, "True"), (*false_exit, "False")] {
        assert_eq!(typed.exprs.ty[exit.as_usize()], bind.return_type);
        assert!(matches!(
            &typed.exprs.const_value[exit.as_usize()],
            Some(ast::ConstValue::ResultAlternative { owner: exit_owner, label: exit_label })
                if exit_owner == &ast::ResultFamilyOwner::Callable(owner.clone())
                    && exit_label.as_str() == label
        ));
    }
}

fn flaw_codes(source: &str) -> Vec<String> {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
    .all_flaws()
    .into_iter()
    .map(|(_, flaw)| flaw.code.slug().to_string())
    .collect()
}

#[test]
fn anonymous_result_alternative_cannot_name_public_parameter_type() {
    let codes = flaw_codes("choose(value) True or False: True\naccept(value True): value");

    assert!(codes.contains(&"type-anonymous-result-not-nameable".to_string()));
}

#[test]
fn anonymous_result_alternative_cannot_name_public_field_type() {
    let codes = flaw_codes("choose(value) True or False: True\nHolder has\n\tvalue True");

    assert!(codes.contains(&"type-anonymous-result-not-nameable".to_string()));
}

#[test]
fn anonymous_result_alternative_cannot_name_public_variant_payload() {
    let codes =
        flaw_codes("choose(value) True or False: True\nEnvelope is Wrapped(value True) or Empty");

    assert!(codes.contains(&"type-anonymous-result-not-nameable".to_string()));
}

#[test]
fn nominal_type_sharing_result_alternative_spelling_remains_nameable() {
    let codes = flaw_codes(
        "True is in 0...1\nchoose(value) True or False: True\naccept(value True) True: value",
    );

    assert!(!codes.contains(&"type-anonymous-result-not-nameable".to_string()));
}

#[test]
fn same_shaped_callable_families_are_incompatible() {
    let typed = transform(
        &TokenCursor::parse_source(
            "first() True or False: True\nsecond() True or False: False\nbad: first() = second()",
        ),
        FileId(0),
        &TransformCtx::new(),
    );
    let owner = |name| match &typed.defs[&DefId(Intern::from_ref(name))].return_type {
        ast::Ty::ResultFamily {
            owner: ast::ResultFamilyOwner::Callable(owner),
            alternatives,
        } => {
            assert_eq!(
                alternatives
                    .iter()
                    .map(|alternative| alternative.label.as_str())
                    .collect::<Vec<_>>(),
                ["True", "False"]
            );
            owner.clone()
        }
        ty => panic!("expected callable result family, got {ty:?}"),
    };
    let first_owner = owner("first");
    let second_owner = owner("second");
    assert_eq!(first_owner.declaration.as_str(), "first");
    assert_eq!(second_owner.declaration.as_str(), "second");
    assert_ne!(first_owner, second_owner);
    let flaws = typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.code.slug().to_string())
        .collect::<Vec<_>>();

    assert!(
        flaws.contains(&"type-result-family-incompatible".to_string()),
        "{flaws:?}"
    );
}

#[test]
fn transparent_forwarding_preserves_result_family_owner() {
    let typed = transform(
        &TokenCursor::parse_source("origin() True or False: True\nforward(): origin()"),
        FileId(0),
        &TransformCtx::new(),
    );
    let origin = &typed.defs[&DefId(Intern::from_ref("origin"))].return_type;
    let forwarded = &typed.defs[&DefId(Intern::from_ref("forward"))].return_type;

    assert_eq!(forwarded, origin);
    assert!(matches!(
        forwarded,
        ast::Ty::ResultFamily {
            owner: ast::ResultFamilyOwner::Callable(owner),
            ..
        } if owner.declaration.as_str() == "origin"
    ));
}

#[test]
fn matching_and_reconstructing_translates_into_wrapper_family() {
    let source = "origin() True or False: True\nwrapper() Yes or No:\n\tif origin() is True\n\t\tYes\n\treturn\n\treturn No\nbad: origin() = wrapper()";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let origin = &typed.defs[&DefId(Intern::from_ref("origin"))].return_type;
    let wrapper = &typed.defs[&DefId(Intern::from_ref("wrapper"))];

    assert!(matches!(
        &wrapper.return_type,
        ast::Ty::ResultFamily {
            owner: ast::ResultFamilyOwner::Callable(owner),
            alternatives,
        } if owner.declaration.as_str() == "wrapper"
            && alternatives.iter().map(|alternative| alternative.label.as_str()).collect::<Vec<_>>()
                == ["Yes", "No"]
    ));
    assert_ne!(&wrapper.return_type, origin);
    assert!(
        wrapper
            .flaws
            .iter()
            .all(|flaw| flaw.code.slug() != "type-result-family-mixed-owners")
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| { flaw.code.slug() == "type-result-family-incompatible" })
    );
}

#[test]
fn mixing_forwarded_and_constructed_families_is_rejected() {
    let source = "origin() True or False: True\nmixed(value):\n\tif value is 0\n\t\torigin()\n\treturn\n\treturn Local";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = &typed.defs[&DefId(Intern::from_ref("mixed"))].flaws;

    assert!(
        flaws
            .iter()
            .any(|flaw| { flaw.code.slug() == "type-result-family-mixed-owners" })
    );
}

#[test]
fn forwarding_two_distinct_families_is_rejected() {
    let source = "first() True or False: True\nsecond() True or False: False\nmixed(value):\n\tif value is 0\n\t\tfirst()\n\treturn\n\treturn second()";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = &typed.defs[&DefId(Intern::from_ref("mixed"))].flaws;

    assert!(
        flaws
            .iter()
            .any(|flaw| { flaw.code.slug() == "type-result-family-mixed-owners" })
    );
}

#[test]
fn local_callable_family_cannot_escape_enclosing_callable() {
    let source = "outer:\n\tinner() True or False: True\n\treturn inner()";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let outer = &typed.defs[&DefId(Intern::from_ref("outer"))];
    let BindBody::Body {
        exprs,
        ret: Some(exit),
    } = &outer.body
    else {
        panic!("expected local callable and return");
    };
    let typecheck::TypedExprKind::Bind { body, .. } = typed.exprs.kind[exprs[0].as_usize()] else {
        panic!("expected local callable bind");
    };
    let expected_owner = ast::ResultFamilyOwner::LocalCallable(ast::BinderId::new(
        0,
        ast::BinderOwner::Expression(exprs[0].0),
    ));

    assert!(matches!(
        &typed.exprs.ty[body.as_usize()],
        ast::Ty::ResultFamily { owner, .. } if owner == &expected_owner
    ));
    assert!(matches!(
        &typed.exprs.ty[exit.as_usize()],
        ast::Ty::ResultFamily { owner, .. } if owner == &expected_owner
    ));
    assert!(
        outer
            .flaws
            .iter()
            .any(|flaw| { flaw.code.slug() == "type-local-result-family-escapes" })
    );
}

#[test]
fn local_callable_family_may_circulate_inside_enclosing_body() {
    let source = "outer:\n\tinner() True or False: True\n\tvalue: inner()\n\treturn 0";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let outer = &typed.defs[&DefId(Intern::from_ref("outer"))];
    let BindBody::Body { exprs, .. } = &outer.body else {
        panic!("expected body");
    };
    let typecheck::TypedExprKind::Bind { body: value, .. } = typed.exprs.kind[exprs[1].as_usize()]
    else {
        panic!("expected local value bind");
    };

    assert!(matches!(
        &typed.exprs.ty[value.as_usize()],
        ast::Ty::ResultFamily {
            owner: ast::ResultFamilyOwner::LocalCallable(_),
            ..
        }
    ));
    assert!(
        outer
            .flaws
            .iter()
            .all(|flaw| { flaw.code.slug() != "type-local-result-family-escapes" })
    );
}

#[test]
fn proof_contract_names_must_refer_to_incoming_values() {
    let typed = transform(
        &TokenCursor::parse_source("check(value) True thus missing < value or False: True"),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = &typed.defs[&DefId(Intern::from_ref("check"))].flaws;

    assert!(
        flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "type-proof-contract-unknown-name")
    );
}

#[test]
fn extern_cannot_assert_untrusted_proof_contract() {
    let typed = transform(
        &TokenCursor::parse_source(
            "Number is in 0...255\ncheck(value Number) True thus value = value or False thus value /= value extern\nresult: check(1)",
        ),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = &typed.defs[&DefId(Intern::from_ref("check"))].flaws;

    assert!(
        flaws
            .iter()
            .any(|flaw| { flaw.code.slug() == "type-extern-proof-contract-untrusted" })
    );
    let BindBody::Expr(call) = typed.defs[&DefId(Intern::from_ref("result"))].body else {
        panic!("expected call body");
    };
    assert!(typed.exprs.result_evidence[call.as_usize()].is_none());
}

#[test]
fn branch_facts_verify_complementary_result_contracts() {
    let source = "less(left, right) True thus left < right or False thus left >= right:\n\tif left < right is True\n\t\tTrue\n\treturn\n\treturn False";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = &typed.defs[&DefId(Intern::from_ref("less"))].flaws;

    assert!(
        flaws
            .iter()
            .all(|flaw| flaw.code.slug() != "type-result-proof-unproven"),
        "{flaws:?}"
    );
}

#[test]
fn construction_without_required_fact_rejects_result_contract() {
    let source = "less(left, right) True thus left < right or False thus left >= right: True";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = &typed.defs[&DefId(Intern::from_ref("less"))].flaws;

    assert!(
        flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "type-result-proof-unproven")
    );
}

#[test]
fn construction_contradicting_known_fact_fails_result_contract() {
    let source = "less(left, right) True thus left >= right or False:\n\tif left < right is True\n\t\tTrue\n\treturn\n\treturn False";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = &typed.defs[&DefId(Intern::from_ref("less"))].flaws;

    assert!(
        flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "type-result-proof-failed")
    );
    assert!(
        flaws
            .iter()
            .all(|flaw| flaw.code.slug() != "type-result-proof-unproven")
    );
}

#[test]
fn repeated_non_equivalent_contracts_for_one_alternative_conflict() {
    let source = "check(value) True thus value = value or True thus value /= value: True";
    let parsed = TokenCursor::parse_source(source);
    assert_eq!(
        parsed.defs[&Intern::from_ref("check")]
            .anonymous_result_alternatives
            .len(),
        2
    );
    assert_ne!(
        parsed.defs[&Intern::from_ref("check")].anonymous_result_alternatives[0]
            .value
            .proposition,
        parsed.defs[&Intern::from_ref("check")].anonymous_result_alternatives[1]
            .value
            .proposition
    );
    let typed = transform(&parsed, FileId(0), &TransformCtx::new());
    let bind = &typed.defs[&DefId(Intern::from_ref("check"))];

    assert!(
        bind.flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "type-result-proof-conflict"),
        "{:?}",
        bind.flaws
    );
    let ast::Ty::ResultFamily { alternatives, .. } = &bind.return_type else {
        panic!("expected result family");
    };
    assert_eq!(alternatives.len(), 1);
}

#[test]
fn verified_contract_substitutes_call_arguments_into_result_evidence() {
    let source = "Number is in 0...255\nless(left Number, right Number) True thus left < right or False thus left >= right:\n\tif left < right is True\n\t\tTrue\n\treturn\n\treturn False\ncomparison: less(1, 2)";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let BindBody::Expr(call) = typed.defs[&DefId(Intern::from_ref("comparison"))].body else {
        panic!("expected call body");
    };
    let evidence = typed.exprs.result_evidence[call.as_usize()]
        .as_ref()
        .unwrap_or_else(|| {
            panic!(
                "missing evidence for {:?} returning {:?}",
                typed.exprs.kind[call.as_usize()],
                typed.defs[&DefId(Intern::from_ref("less"))].return_type
            )
        });

    assert!(matches!(
        &evidence.owner,
        ast::ResultFamilyOwner::Callable(owner) if owner.declaration.as_str() == "less"
    ));
    assert!(matches!(
        evidence.alternatives.as_slice(),
        [
            typecheck::AlternativeEvidence {
                label: true_label,
                proposition: typecheck::EvidenceProposition::IntegerComparison {
                    comparison: typecheck::MathematicalComparison::Less,
                    ..
                },
            },
            typecheck::AlternativeEvidence {
                label: false_label,
                proposition: typecheck::EvidenceProposition::IntegerComparison {
                    comparison: typecheck::MathematicalComparison::GreaterOrEqual,
                    ..
                },
            },
        ] if true_label.as_str() == "True" && false_label.as_str() == "False"
    ));
}

#[test]
fn transparent_forwarding_substitutes_outer_call_arguments_into_evidence() {
    let source = "Number is in 0...255\nless(left Number, right Number) True thus left < right or False thus left >= right:\n\tif left < right is True\n\t\tTrue\n\treturn\n\treturn False\nforward(left Number, right Number): less(left, right)\ncomparison: forward(1, 2)";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let BindBody::Expr(call) = typed.defs[&DefId(Intern::from_ref("comparison"))].body else {
        panic!("expected call body");
    };
    let typecheck::TypedExprKind::FnCall {
        args: Some(arguments),
        ..
    } = &typed.exprs.kind[call.as_usize()]
    else {
        panic!("expected forwarded call");
    };
    let evidence = typed.exprs.result_evidence[call.as_usize()]
        .as_ref()
        .expect("forwarded evidence");

    assert!(evidence.alternatives.iter().all(|alternative| matches!(
        alternative.proposition,
        typecheck::EvidenceProposition::IntegerComparison { lhs, rhs, .. }
            if lhs == arguments[0] && rhs == arguments[1]
    )));
}

#[test]
fn inferred_generic_identity_preserves_result_family_and_evidence() {
    let source = "Number is in 0...255\nless(left Number, right Number) True thus left < right or False thus left >= right:\n\tif left < right is True\n\t\tTrue\n\treturn\n\treturn False\nidentity(value x): value\ncomparison: identity(less(1, 2))";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let BindBody::Expr(identity_call) = typed.defs[&DefId(Intern::from_ref("comparison"))].body
    else {
        panic!("expected identity call");
    };
    let typecheck::TypedExprKind::FnCall {
        args: Some(arguments),
        ..
    } = &typed.exprs.kind[identity_call.as_usize()]
    else {
        panic!("expected call arguments");
    };
    let source_result = arguments[0];

    assert!(
        matches!(
            &typed.exprs.ty[identity_call.as_usize()],
            ast::Ty::ResultFamily {
                owner: ast::ResultFamilyOwner::Callable(owner),
                ..
            } if owner.declaration.as_str() == "less"
        ),
        "{:?}",
        typed.exprs.ty[identity_call.as_usize()]
    );
    assert_eq!(
        typed.exprs.result_evidence[identity_call.as_usize()],
        typed.exprs.result_evidence[source_result.as_usize()]
    );
    assert!(typed.exprs.result_evidence[identity_call.as_usize()].is_some());
}
