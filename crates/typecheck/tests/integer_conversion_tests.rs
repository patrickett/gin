mod support;

use internment::Intern;
use support::{check, flaw_codes};
use typecheck::{BindBody, DefId, TypedExprKind};

#[test]
fn exact_anonymous_value_proves_nominal_conversion_containment() {
    let typed = check("Tiny is in 0 to 15\nconverted: 10 as Tiny");
    let codes = flaw_codes(&typed);
    assert!(
        !codes
            .iter()
            .any(|code| code.starts_with("type-integer-narrowing"))
    );
    let BindBody::Expr(converted) = typed.defs[&DefId::named(Intern::from_ref("converted"))].body
    else {
        panic!("expected cast body");
    };
    assert!(matches!(
        &typed.exprs.kind[converted.as_usize()],
        TypedExprKind::Cast { ty: ast::Ty::Named { name, .. }, .. } if name.as_str() == "Tiny"
    ));
}

#[test]
fn exact_anonymous_value_outside_nominal_validity_fails_conversion() {
    let typed = check("Tiny is in 0 to 15\nconverted: 20 as Tiny");
    let codes = flaw_codes(&typed);
    assert!(
        codes
            .iter()
            .any(|code| code == "type-integer-narrowing-failed"),
        "{codes:?}"
    );
}

#[test]
fn symbolic_anonymous_domain_leaves_nominal_conversion_unproven() {
    let typed =
        check("Tiny is in 0 to 15\nsource() is < limit: return 0\nconverted: source() as Tiny");
    let codes = flaw_codes(&typed);
    assert!(
        codes
            .iter()
            .any(|code| code == "type-integer-narrowing-unproven"),
        "{codes:?}"
    );
}

#[test]
fn a_loop_conversion_cannot_reuse_the_counters_initial_constant() {
    let typed = check(
        "Count is in 0 to 255\nZero is in 0 to 0\nZero has\n    release(self):\n        return\nrun:\n    index is in Count: 0\n    while index < 4 do\n        value: index as Zero\n        value.release\n        index :: (next is in Count: index + 1)\n    return\n",
    );
    let codes = flaw_codes(&typed);
    assert!(
        codes
            .iter()
            .any(|code| code.starts_with("type-integer-narrowing-")),
        "{codes:?}"
    );
}

#[test]
fn named_integer_range_guards_prove_conversions_without_erasing_the_name() {
    for body in [
        "when value is in 0 to 255 then value as Byte else 0",
        "when value is\n    in 0 to 255 then value as Byte\n    else 0",
    ] {
        let typed = check(&format!(
            "Signed is in -9223372036854775808 to 9223372036854775807\n{}convert(value Signed) Byte: {return_prefix}{body}\n",
            test_fixtures::preludes::BYTE_PRELUDE,
            return_prefix = if body.starts_with('\n') {
                ""
            } else {
                "return "
            }
        ));
        let codes = flaw_codes(&typed);
        assert!(
            !codes
                .iter()
                .any(|code| code.starts_with("type-integer-narrowing")),
            "{body}: {codes:?}"
        );
        let source = typed
            .exprs
            .kind
            .iter()
            .find_map(|kind| match kind {
                TypedExprKind::Cast {
                    expr,
                    ty: ast::Ty::Named { name, .. },
                } if name.as_str() == "Byte" => Some(*expr),
                _ => None,
            })
            .unwrap();
        assert!(
            matches!(&typed.exprs.ty[source.as_usize()], ast::Ty::Named { name, .. } if name.as_str() == "Signed")
        );
        assert!(matches!(
            typed.exprs.integer_knowledge_of(source),
            Some(Some(ast::integer::IntegerKnowledge::Domain(domain)))
                if domain.storage_hull().is_some_and(|hull| hull.min() == 0.into() && hull.max() == 255.into())
        ));
    }
}

#[test]
fn range_evidence_does_not_escape_the_matching_branch() {
    for body in [
        "value as Byte",
        "when value is in 0 to 255 then 0 else value as Byte",
        "when value is in 0 to 255 or value is in 256 to 511 then value as Byte else 0",
    ] {
        let typed = check(&format!(
            "Signed is in -9223372036854775808 to 9223372036854775807\n{}convert(value Signed) Byte: {return_prefix}{body}\n",
            test_fixtures::preludes::BYTE_PRELUDE,
            return_prefix = if body.starts_with('\n') {
                ""
            } else {
                "return "
            }
        ));
        assert!(
            flaw_codes(&typed)
                .iter()
                .any(|code| code.starts_with("type-integer-narrowing")),
            "{body}"
        );
    }
}

#[test]
fn named_type_range_is_available_without_a_guard() {
    let typed = check(&format!(
        "Small is in 0 to 15\nSmall has\n    finish(self):\n        return\n{}convert(value Small) Byte: return value as Byte",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(flaw_codes(&typed).is_empty(), "{:?}", flaw_codes(&typed));
}

#[test]
fn range_evidence_is_invalidated_by_reassignment_and_mutable_calls() {
    for action in ["value :: replace(value, 300)", "touch(value)"] {
        let source = format!(
            "Word is in 0 to 511\nWord.finish(self): return\n{}replace(previous Word, next Word) Word: return next\ntouch(value mut Word):\n    value:: replace(value, 300)\n    return\nconvert(input Word) Byte:\n    value Word: input\n    when value is in 0 to 255 then\n        {action}\n        return value as Byte\n    return 0\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let typed = check(&source);
        assert!(
            flaw_codes(&typed)
                .iter()
                .any(|code| code.starts_with("type-integer-narrowing")),
            "{source}: {:?}",
            flaw_codes(&typed)
        );
    }
}

#[test]
fn a_guard_before_a_mutating_condition_does_not_prove_the_updated_value() {
    let typed = check(&format!(
        "Word is in 0 to 511\nWord.finish(self): return\n{}replace(previous Word, next Word) Word: return next\ntouch(value mut Word) Ready or Failed:\n    value:: replace(value, 300)\n    return Ready\nconvert(input Word) Byte:\n    value Word: input\n    return when value is in 0 to 255 and touch(value) is Ready then value as Byte else 0\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code.starts_with("type-integer-narrowing")),
        "{:?}",
        flaw_codes(&typed)
    );
}

#[test]
fn a_new_guard_after_mutation_proves_the_current_value() {
    let typed = check(&format!(
        "Word is in 0 to 511\nWord.finish(self): return\n{}touch(value mut Word):\n    value:: replace(value, 300)\n    return\nconvert(input Word) Byte:\n    value Word: input\n    touch(value)\n    return when value is in 0 to 255 then value as Byte else 0\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        !flaw_codes(&typed)
            .iter()
            .any(|code| code.starts_with("type-integer-narrowing")),
        "{:?}",
        flaw_codes(&typed)
    );
}

#[test]
fn narrowed_named_value_keeps_its_evidence_when_moved_to_a_local() {
    let typed = check(&format!(
        "Word is in 0 to 511\nWord.finish(self): return\n{}convert(value Word) Byte:\n    when value is in 0 to 255 then\n        moved: value\n        return moved as Byte\n    return 0\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        !flaw_codes(&typed)
            .iter()
            .any(|code| code.starts_with("type-integer-narrowing")),
        "{:?}",
        flaw_codes(&typed)
    );
}

#[test]
fn named_comparison_without_a_proof_does_not_narrow_a_named_integer() {
    let typed = check(&format!(
        "Word is in 0 to 511\nWord.finish(self): return\n{}less(left ref Word, right Word) True or False: return True\nconvert(value Word) Byte: return when less(value, 256) is True then value as Byte else 0\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code.starts_with("type-integer-narrowing")),
        "{:?}",
        flaw_codes(&typed)
    );
}

#[test]
fn loop_body_uses_its_guard_instead_of_an_earlier_value_range() {
    for (guard, should_prove) in [("0 to 255", true), ("0 to 511", false)] {
        let source = format!(
            "Word is in 0 to 511\nWord.finish(self): return\n{}convert(input Word):\n    value Word: input\n    when value is in 0 to 255 then\n        while value is in {guard} do\n            narrowed Byte: value as Byte\n            value :: 300\n        return\n    return\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let typed = check(&source);
        let rejected = flaw_codes(&typed)
            .iter()
            .any(|code| code.starts_with("type-integer-narrowing"));
        assert_eq!(
            !rejected,
            should_prove,
            "{source}: {:?}",
            flaw_codes(&typed)
        );
    }
}

#[test]
fn nonnegative_bit_masks_prove_narrowing() {
    for expression in ["value & 255", "255 & value", "(value >> 8) & 255"] {
        let typed = check(&format!(
            "{}byte(value is in 0 to 4294967295) Byte: return ({expression}) as Byte\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ));
        assert!(
            typed.all_flaws().is_empty(),
            "{expression}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn wider_or_negative_masks_do_not_prove_byte_narrowing() {
    for mask in ["511", "-1"] {
        let typed = check(&format!(
            "{}byte(value is in 0 to 4294967295) Byte: return (value & {mask}) as Byte\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ));
        assert!(
            flaw_codes(&typed)
                .iter()
                .any(|code| code.starts_with("type-integer-narrowing"))
        );
    }
}

#[test]
fn narrowing_requires_the_complete_target_domain_including_exclusions() {
    let declarations =
        "Nonzero is in -128 to 127 and /= 0\nNonzero has\n    finish(self):\n        return\n";
    for (body, accepted) in [
        ("1 as Nonzero", true),
        ("0 as Nonzero", false),
        (
            "when value is in Nonzero then value as Nonzero else 1",
            true,
        ),
        (
            "when value is in -128 to 127 then value as Nonzero else 1",
            false,
        ),
        (
            "when value is in -128 to 127 and not (value = 0) then value as Nonzero else 1",
            true,
        ),
    ] {
        let source = format!(
            "{declarations}convert(value is in -32768 to 32767) Nonzero: {return_prefix}{body}\n",
            return_prefix = if body.starts_with('\n') {
                ""
            } else {
                "return "
            }
        );
        let typed = check(&source);
        if accepted {
            assert!(
                typed.all_flaws().is_empty(),
                "{source}: {:?}",
                typed.all_flaws()
            );
        } else {
            assert!(
                flaw_codes(&typed)
                    .iter()
                    .any(|code| code.starts_with("type-integer-narrowing-")),
                "{source}: {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn guarded_parameter_narrowing_uses_current_occurrence_evidence() {
    let declarations = "Word is in 0 to 511\nWord has\n    finish(self):\n        return\nidentity(input is in 0 to 255) is in 0 to 255: return input\ntouch(value mut Word): return 0\n";
    for (body, accepted) in [
        (
            "    when value is in 0 to 255 then\n        return identity(value)\n    return 0",
            true,
        ),
        (
            "    when value is in 0 to 255 then\n        value :: 300\n        return identity(value)\n    return 0",
            false,
        ),
        (
            "    when value is in 0 to 255 then\n        touch(value)\n        return identity(value)\n    return 0",
            false,
        ),
        (
            "    when value is in 0 to 255 then\n        return 0\n    return identity(value)",
            false,
        ),
    ] {
        let source = format!(
            "{declarations}convert(input Word) is in 0 to 255:\n    value Word: input\n{body}\n"
        );
        let typed = check(&source);
        if accepted {
            assert!(
                typed.all_flaws().is_empty(),
                "{source}: {:?}",
                typed.all_flaws()
            );
        } else {
            assert!(!typed.all_flaws().is_empty(), "{source}");
        }
    }
}

#[test]
fn anonymous_return_refinements_validate_each_early_exit() {
    for body in [
        "value",
        "\n    when value is in 0 to 255 then\n        return 256\n    return 0",
        "\n    when value is in 0 to 511 then\n        return value\n    return 0",
    ] {
        let source = format!("convert(value is in 0 to 511) is in 0 to 255:return {body}\n");
        let typed = check(&source);
        assert!(
            !typed.all_flaws().is_empty(),
            "invalid return accepted: {source}"
        );
    }
}

#[test]
fn anonymous_return_refinements_validate_each_when_arm() {
    for (body, accepted) in [
        ("when value is in 0 to 255 then value else 0", true),
        ("when value is in 0 to 255 then 0 else value", false),
        ("when value is in 0 to 511 then value else 0", false),
        ("when value is in 0 to 255 then value else 256", false),
    ] {
        let source = format!(
            "convert(value is in 0 to 511) is in 0 to 255: {return_prefix}{body}\n",
            return_prefix = if body.starts_with('\n') {
                ""
            } else {
                "return "
            }
        );
        let typed = check(&source);
        assert_eq!(
            typed.all_flaws().is_empty(),
            accepted,
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn guarded_parameter_narrowing_preserves_exclusions() {
    let declarations = "Nonzero is in -128 to 127 and /= 0\nNonzero has\n    finish(self):\n        return\nidentity(input is in -128 to 127 and /= 0) is in -128 to 127 and /= 0: return input\n";
    for (guard, accepted) in [
        ("value is in Nonzero", true),
        ("value is in -128 to 127 and not (value = 0)", true),
        ("value is in -128 to 127", false),
    ] {
        let source = format!(
            "{declarations}convert(value is in -32768 to 32767) is in -128 to 127 and /= 0: return when {guard} then identity(value) else 1\n"
        );
        let typed = check(&source);
        assert_eq!(
            typed.all_flaws().is_empty(),
            accepted,
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn borrowed_comparison_proofs_do_not_narrow_reassigned_values() {
    for action in ["value:: 511", "touch(value)"] {
        let source = format!(
            "Byte is in 0 to 255\nByte has finish(self): return\nless(left ref is in 0 to 511, right is in 0 to 511) True thus left < right or False thus left >= right:\n    when left < right then\n        return True\n    return False\nidentity(value x): return value\ntouch(value mut is in 0 to 511):\n    value:: 511\n    return\nnarrow(value is in 0 to 511) Byte:\n    decision: identity(less(value, 256))\n    {action}\n    when decision is True then\n        return value as Byte\n    return 0\n"
        );
        let typed = check(&source);
        assert!(
            flaw_codes(&typed)
                .iter()
                .any(|code| code == "type-integer-narrowing-failed"),
            "{source}: {:?}",
            flaw_codes(&typed)
        );
    }
}
