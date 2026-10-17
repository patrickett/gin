mod support;

use internment::Intern;
use parser::cursor::TokenCursor;
use support::{
    check, check_lexed, check_parsed, def_flaw_codes, expect_flaw_code, expect_no_flaws, flaw_codes,
};
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, FileId};

#[test]
fn explicit_anonymous_result_surface_has_callable_owner() {
    let source = "choose(value is in 0 to 1) True or False: return True";
    let typed = check(source);
    let ty = &typed.defs[&DefId::named(Intern::from_ref("choose"))].return_type;
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
    let BindBody::Expr(body) = typed.defs[&DefId::named(Intern::from_ref("choose"))].body else {
        panic!("expected expression body");
    };
    assert_eq!(
        typed.exprs.facts.const_facts.const_value[body.as_usize()],
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
    let typed = check("choose: Ready");
    let bind = &typed.defs[&DefId::named(Intern::from_ref("choose"))];
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
        &typed.exprs.facts.const_facts.const_value[body.as_usize()],
        Some(ast::ConstValue::ResultAlternative { owner: value_owner, label })
            if value_owner == &ast::ResultFamilyOwner::Callable(owner.clone())
                && label.as_str() == "Ready"
    ));
}

#[test]
fn inferred_branch_exits_extend_one_callable_family() {
    let source =
        "choose(value is in 0 to 1):\n\twhen value is 0 then\n\t\treturn True\n\treturn False";
    let typed = check(source);
    let bind = &typed.defs[&DefId::named(Intern::from_ref("choose"))];
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
    let true_exit = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            typecheck::TypedExprKind::Return { value, .. } => *value,
            _ => None,
        })
        .expect("taken exit");
    assert!(matches!(
        typed.exprs.kind[exprs[0].as_usize()],
        typecheck::TypedExprKind::When(_)
    ));
    for (exit, label) in [(true_exit, "True"), (*false_exit, "False")] {
        assert_eq!(typed.exprs.ty[exit.as_usize()], bind.return_type);
        assert!(matches!(
            &typed.exprs.facts.const_facts.const_value[exit.as_usize()],
            Some(ast::ConstValue::ResultAlternative { owner: exit_owner, label: exit_label })
                if exit_owner == &ast::ResultFamilyOwner::Callable(owner.clone())
                    && exit_label.as_str() == label
        ));
    }
}

#[test]
fn anonymous_result_alternative_cannot_name_public_parameter_type() {
    let codes = flaw_codes(&check_parsed(
        "choose(value) True or False: return True\naccept(value True): return value",
    ));

    assert!(codes.contains(&"type-anonymous-result-not-nameable".to_string()));
}

#[test]
fn anonymous_result_alternative_cannot_name_public_field_type() {
    let codes = flaw_codes(&check_parsed(
        "choose(value) True or False: return True\nHolder has\n\tvalue True",
    ));

    assert!(codes.contains(&"type-anonymous-result-not-nameable".to_string()));
}

#[test]
fn anonymous_result_alternative_cannot_name_public_variant_payload() {
    let codes = flaw_codes(&check_parsed(
        "choose(value) True or False: return True\nEnvelope is Wrapped(value True) or Empty",
    ));

    assert!(codes.contains(&"type-anonymous-result-not-nameable".to_string()));
}

#[test]
fn nominal_type_sharing_result_alternative_spelling_remains_nameable() {
    let codes = flaw_codes(&check_parsed(
        "True is in 0 to 1\nchoose(value) True or False: return True\naccept(value True) True: return value",
    ));

    assert!(!codes.contains(&"type-anonymous-result-not-nameable".to_string()));
}

#[test]
fn same_shaped_callable_families_are_incompatible() {
    let typed = check(
        "first() True or False: return True\nsecond() True or False: return False\nbad: first() = second()",
    );
    let owner = |name| match &typed.defs[&DefId::named(Intern::from_ref(name))].return_type {
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
    let codes = flaw_codes(&typed);

    assert!(
        codes.contains(&"type-result-family-incompatible".to_string()),
        "{codes:?}"
    );
}

#[test]
fn transparent_forwarding_preserves_result_family_owner() {
    let typed = check("origin() True or False: return True\nforward(): return origin()");
    let origin = &typed.defs[&DefId::named(Intern::from_ref("origin"))].return_type;
    let forwarded = &typed.defs[&DefId::named(Intern::from_ref("forward"))].return_type;

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
    let source = "origin() True or False: return True\nwrapper() Yes or No:\n\twhen origin() is True then\n\t\treturn Yes\n\treturn No\nbad: origin() = wrapper()";
    let typed = check(source);
    let origin = &typed.defs[&DefId::named(Intern::from_ref("origin"))].return_type;
    let wrapper = &typed.defs[&DefId::named(Intern::from_ref("wrapper"))];

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
    let wrapper_codes = def_flaw_codes(&typed, "wrapper");
    assert!(
        wrapper_codes
            .iter()
            .all(|code| code != "type-result-family-mixed-owners")
    );
    assert!(flaw_codes(&typed).contains(&"type-result-family-incompatible".to_string()));
}

#[test]
fn mixing_forwarded_and_constructed_families_is_rejected() {
    let source = "origin() True or False: return True\nmixed(value is in 0 to 1):\n\twhen value is 0 then\n\t\treturn origin()\n\treturn Local";
    let typed = check(source);
    let codes = def_flaw_codes(&typed, "mixed");

    assert!(codes.contains(&"type-result-family-mixed-owners".to_string()));
}

#[test]
fn forwarding_two_distinct_families_is_rejected() {
    let source = "first() True or False: return True\nsecond() True or False: return False\nmixed(value is in 0 to 1):\n\twhen value is 0 then\n\t\treturn first()\n\treturn second()";
    let typed = check(source);
    let codes = def_flaw_codes(&typed, "mixed");

    assert!(codes.contains(&"type-result-family-mixed-owners".to_string()));
}

#[test]
fn local_callable_family_cannot_escape_enclosing_callable() {
    let source = "outer:\n\tinner() True or False: return True\n\treturn inner()";
    let typed = check(source);
    let outer = &typed.defs[&DefId::named(Intern::from_ref("outer"))];
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
    let codes = def_flaw_codes(&typed, "outer");
    assert!(codes.contains(&"type-local-result-family-escapes".to_string()));
}

#[test]
fn local_callable_family_may_circulate_inside_enclosing_body() {
    let source = "outer:\n\tinner() True or False: return True\n\tvalue: inner()\n\treturn 0";
    let typed = check(source);
    let outer = &typed.defs[&DefId::named(Intern::from_ref("outer"))];
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
    let codes = def_flaw_codes(&typed, "outer");
    assert!(
        codes
            .iter()
            .all(|code| code != "type-local-result-family-escapes")
    );
}

#[test]
fn proof_contract_names_must_refer_to_incoming_values() {
    let typed = check("check(value) True thus missing < value or False: return True");
    let codes = def_flaw_codes(&typed, "check");

    assert!(codes.contains(&"type-proof-contract-unknown-name".to_string()));
}

#[test]
fn proof_contracts_require_a_gin_body() {
    let parsed = TokenCursor::parse_source(
        "Number is in 0 to 255\ncheck(value Number) True thus value = value or False extern\n",
    );
    assert!(
        parsed
            .parse_warnings
            .iter()
            .any(|flaw| flaw.code.slug() == "parse-removed-extern")
    );
}

#[test]
fn branch_facts_verify_complementary_result_contracts() {
    let source = "less(left, right) True thus left < right or False thus left >= right:\n\twhen left < right then\n\t\treturn True\n\treturn False";
    let typed = check(source);
    let codes = def_flaw_codes(&typed, "less");

    assert!(
        codes
            .iter()
            .all(|code| code != "type-result-proof-unproven"),
        "{codes:?}"
    );
}

#[test]
fn construction_without_required_fact_rejects_result_contract() {
    let source =
        "less(left, right) True thus left < right or False thus left >= right: return True";
    let typed = check(source);
    let codes = def_flaw_codes(&typed, "less");

    assert!(codes.contains(&"type-result-proof-unproven".to_string()));
}

#[test]
fn construction_contradicting_known_fact_fails_result_contract() {
    let source = "less(left, right) True thus left >= right or False:\n\twhen left < right then\n\t\treturn True\n\treturn False";
    let typed = check(source);
    let codes = def_flaw_codes(&typed, "less");

    assert!(codes.contains(&"type-result-proof-failed".to_string()));
    assert!(
        codes
            .iter()
            .all(|code| code != "type-result-proof-unproven")
    );
}

#[test]
fn repeated_non_equivalent_contracts_for_one_alternative_conflict() {
    let source = "check(value) True thus value = value or True thus value /= value: return True";
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
    let bind = &typed.defs[&DefId::named(Intern::from_ref("check"))];

    assert!(def_flaw_codes(&typed, "check").contains(&"type-result-proof-conflict".to_string()));
    let ast::Ty::ResultFamily { alternatives, .. } = &bind.return_type else {
        panic!("expected result family");
    };
    assert_eq!(alternatives.len(), 1);
}

#[test]
fn verified_contract_substitutes_call_arguments_into_result_evidence() {
    let source = "Number is in 0 to 255\nless(left Number, right Number) True thus left < right or False thus left >= right:\n\twhen left < right then\n\t\treturn True\n\treturn False\ncomparison: less(1, 2)";
    let typed = check(source);
    let BindBody::Expr(call) = typed.defs[&DefId::named(Intern::from_ref("comparison"))].body
    else {
        panic!("expected call body");
    };
    let evidence = typed.exprs.result_evidence_of(call).unwrap_or_else(|| {
        panic!(
            "missing evidence for {:?} returning {:?}",
            typed.exprs.kind[call.as_usize()],
            typed.defs[&DefId::named(Intern::from_ref("less"))].return_type
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
    let source = "Number is in 0 to 255\nless(left Number, right Number) True thus left < right or False thus left >= right:\n\twhen left < right then\n\t\treturn True\n\treturn False\nforward(left Number, right Number): return less(left, right)\ncomparison: forward(1, 2)";
    let typed = check(source);
    let BindBody::Expr(call) = typed.defs[&DefId::named(Intern::from_ref("comparison"))].body
    else {
        panic!("expected call body");
    };
    let typecheck::TypedExprKind::FnCall {
        args: Some(arguments),
        ..
    } = &typed.exprs.kind[call.as_usize()]
    else {
        panic!("expected forwarded call");
    };
    let evidence = typed
        .exprs
        .result_evidence_of(call)
        .expect("forwarded evidence");

    assert!(evidence.alternatives.iter().all(|alternative| matches!(
        alternative.proposition,
        typecheck::EvidenceProposition::IntegerComparison { lhs, rhs, .. }
            if lhs == arguments[0] && rhs == arguments[1]
    )));
}

#[test]
fn inferred_generic_identity_preserves_result_family_and_evidence() {
    let source = "less(left is in 0 to 255, right is in 0 to 255) True thus left < right or False thus left >= right:\n\twhen left < right then\n\t\treturn True\n\treturn False\nidentity(value x): return value\ncomparison: identity(less(1, 2))";
    let typed = check(source);
    let BindBody::Expr(identity_call) =
        typed.defs[&DefId::named(Intern::from_ref("comparison"))].body
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
    expect_no_flaws(&typed);
    assert!(typed.defs.values().any(
        |bind| bind.name.as_str().starts_with("identity$specialized$")
            && !matches!(bind.body, BindBody::Unlowered)
    ));

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
        typed.exprs.facts.semantic_facts.result_evidence[identity_call.as_usize()],
        typed.exprs.facts.semantic_facts.result_evidence[source_result.as_usize()]
    );
    assert!(typed.exprs.facts.semantic_facts.result_evidence[identity_call.as_usize()].is_some());
}

#[test]
fn generic_forwarding_uses_the_returned_parameters_proof() {
    let source = "Byte is in 0 to 255\nByte has finish(self): return\nless(left is in 0 to 511, right is in 0 to 511) True thus left < right or False thus left >= right:\n    when left < right then\n        return True\n    return False\nsecond(first x, last x): return last\nnarrow(value is in 0 to 511) Byte:\n    zero is in 0 to 511: 0\n    when second(less(value + 0, 256), less(zero, 1)) is True then\n        return value as Byte\n    return 0\n";
    let typed = check_parsed(source);
    let codes = flaw_codes(&typed);
    let (call, arguments) = typed
        .exprs
        .kind
        .iter()
        .enumerate()
        .find_map(|(index, kind)| match kind {
            typecheck::TypedExprKind::FnCall {
                target,
                args: Some(arguments),
                ..
            } if target.0.as_str().starts_with("second$specialized$") => Some((index, arguments)),
            _ => None,
        })
        .expect("specialized forwarding call");
    let evidence = &typed.exprs.facts.semantic_facts.result_evidence;
    assert!(evidence[call].is_some());
    assert_eq!(evidence[call], evidence[arguments[1].as_usize()]);
    assert_ne!(evidence[call], evidence[arguments[0].as_usize()]);
    assert!(
        codes.contains(&"type-integer-narrowing-failed".to_string())
            || codes.contains(&"type-integer-narrowing-unproven".to_string()),
        "{codes:?}"
    );
    assert!(
        codes
            .iter()
            .all(|code| code == "type-integer-narrowing-failed"
                || code == "type-integer-narrowing-unproven"),
        "{codes:?}"
    );
}

#[test]
fn generic_specializations_keep_distinct_result_family_owners() {
    let source = "first(left is in 0 to 255, right is in 0 to 255) True thus left < right or False thus left >= right:\n    when left < right then\n        return True\n    return False\nsecond(left is in 0 to 255, right is in 0 to 255) True thus left < right or False thus left >= right:\n    when left < right then\n        return True\n    return False\nidentity(value x): return value\nbad: identity(first(1, 2)) = identity(second(1, 2))\n";
    let typed = check(source);
    expect_flaw_code(&typed, "type-result-family-incompatible");
    let specializations = typed
        .defs
        .values()
        .filter(|bind| bind.name.as_str().starts_with("identity$specialized$"))
        .collect::<Vec<_>>();
    assert_eq!(specializations.len(), 2);
    assert!(
        specializations
            .iter()
            .all(|bind| !matches!(bind.body, BindBody::Unlowered))
    );
    assert_ne!(
        specializations[0].return_type,
        specializations[1].return_type
    );
}

#[test]
fn generic_result_parameters_do_not_capture_unrelated_parameter_names() {
    let source = "Byte is in 0 to 255\nByte has finish(self): return\nless(left is in 0 to 511, right is in 0 to 511) True thus left < right or False thus left >= right:\n    when left < right then\n        return True\n    return False\nconsume(proof x, left is in 0 to 511) Byte:\n    when proof is True then\n        return left as Byte\n    return 0\nrun(left is in 0 to 511) Byte:\n    zero is in 0 to 511: 0\n    return consume(less(zero, 1), left)\n";
    let codes = flaw_codes(&check_parsed(source));
    assert!(
        codes
            .iter()
            .any(|code| code == "type-integer-narrowing-failed"
                || code == "type-integer-narrowing-unproven"),
        "{codes:?}"
    );
}

#[test]
fn integer_refinement_parameters_preserve_borrowing_in_the_signature() {
    for (convention, mutable) in [("ref", false), ("mut", true)] {
        for domain in ["0 to 511", "Domain"] {
            let source = format!(
                "Domain is in 0 to 511\nDomain has finish(self): return\nread(value {convention} is in {domain}) is in 0 to 511: return deref value\n"
            );
            let typed = check(&source);
            assert!(
                typed.all_flaws().is_empty(),
                "{source}: {:?}",
                typed.all_flaws()
            );
            let ty = &typed.defs[&DefId::named(Intern::from_ref("read"))].params[0].1;
            assert!(
                matches!(ty, ast::Ty::Ref {inner, mutable: actual} if *actual == mutable && inner.anonymous_integer_validity().is_some()),
                "{ty:?}"
            );
        }
    }
}

enum Expectation {
    Empty,
    Contains(&'static str),
}

const CASES: &[(&str, &str, &str, Expectation)] = &[
    (
        "nested_conditional_result_construction_checks_its_contract",
        "less(left, right) True thus left >= right or False:\n    when left < right then\n        when left < right then\n            return True\n        return False\n    return False\n",
        "less",
        Expectation::Contains("type-result-proof-failed"),
    ),
    (
        "nested_conditional_result_keeps_valid_enclosing_facts",
        "less(left, right) True thus left < right or False:\n    when left < right then\n        when left < right then\n            return True\n        return False\n    return False\n",
        "less",
        Expectation::Empty,
    ),
    (
        "nested_conditional_facts_do_not_prove_an_outer_result",
        "equal(left, right) True thus left = right or False:\n    when left < right then\n        when left = right then\n            return False\n        return False\n    return True\n",
        "equal",
        Expectation::Contains("type-result-proof-unproven"),
    ),
    (
        "conditional_expression_result_construction_checks_its_contract",
        "less(left, right) True thus left >= right or False: return when left < right then True else False\n",
        "less",
        Expectation::Contains("type-result-proof-failed"),
    ),
    (
        "conditional_expression_result_proves_taken_and_fallthrough_contracts",
        "less(left, right) True thus left < right or False thus left >= right: return when left < right then True else False\n",
        "less",
        Expectation::Empty,
    ),
    (
        "conditional_expression_arm_facts_do_not_prove_other_arms",
        "equal(left, right) True thus left = right or False: return when left = right then False else True\n",
        "equal",
        Expectation::Contains("type-result-proof-failed"),
    ),
    (
        "nested_result_proof_follows_a_cycle_of_equal_names",
        "equal(left, right, third) True thus left = third or False:\n    when left = right then\n        when right = third then\n            when third = left then\n                return True\n            return False\n        return False\n    return False\n",
        "equal",
        Expectation::Empty,
    ),
    (
        "cyclic_equal_names_do_not_prove_an_unrelated_name",
        "equal(left, right, third) True thus left = third or False:\n    when left = right then\n        when right = left then\n            return True\n        return False\n    return False\n",
        "equal",
        Expectation::Contains("type-result-proof-unproven"),
    ),
    (
        "while_loop_result_exit_checks_its_contract",
        "less(left, right) True thus left >= right or False:\n    while left < right do\n        when left < right then\n            return True\n    return False\n",
        "less",
        Expectation::Contains("type-result-proof-failed"),
    ),
    (
        "for_loop_result_exit_checks_its_contract",
        "less(left, right) True thus left >= right or False:\n    for item in 0 to 1\n        when left < right then\n            return True\n    return False\n",
        "less",
        Expectation::Contains("type-result-proof-failed"),
    ),
    (
        "while_loop_result_exit_keeps_its_taken_condition",
        "less(left, right) True thus left < right or False:\n    while left < right do\n        when right >= left then\n            return True\n    return False\n",
        "less",
        Expectation::Empty,
    ),
];

#[test]
fn conditional_result_proof_cases() {
    for &(name, source, def, ref expectation) in CASES {
        let typed = check_lexed(source);
        let codes = def_flaw_codes(&typed, def);
        match expectation {
            Expectation::Empty => assert!(codes.is_empty(), "case `{name}` failed: {codes:?}"),
            Expectation::Contains(code) => assert!(
                codes.contains(&code.to_string()),
                "case `{name}` failed: {codes:?}"
            ),
        }
    }
}

#[test]
fn condition_form_result_proofs_use_negated_fallthrough_facts() {
    for (relation, expected_failure) in [("<", false), (">=", true)] {
        let source = format!(
            "less(left, right) True thus left {relation} right or False: return when not left < right then False else True\n"
        );
        let typed = check_lexed(&source);
        let codes = def_flaw_codes(&typed, "less");
        if expected_failure {
            assert!(
                codes.contains(&"type-result-proof-failed".to_string()),
                "{codes:?}"
            );
        } else {
            assert!(codes.is_empty(), "{codes:?}");
        }
    }
}

#[test]
fn loop_body_facts_do_not_prove_a_result_after_the_loop() {
    for opening in ["while left < right do", "for item in 0 to 1"] {
        let source = format!(
            "equal(left, right) True thus left = right or False:\n    {opening}\n        when left = right then\n            return False\n    return True\n"
        );
        let typed = check_lexed(&source);
        let codes = def_flaw_codes(&typed, "equal");
        assert!(
            codes.contains(&"type-result-proof-unproven".to_string()),
            "{codes:?}"
        );
    }
}

#[test]
fn for_loop_result_exit_keeps_enclosing_branch_facts() {
    let source = "less(left, right) True thus left < right or False:\n    when left < right then\n        for item in 0 to 1\n            when right >= left then\n                return True\n        return False\n    return False\n";
    let typed = check_lexed(source);
    let codes = def_flaw_codes(&typed, "less");
    assert!(codes.is_empty(), "{codes:?}");
}
