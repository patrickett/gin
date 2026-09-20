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
    let BindBody::Expr(converted) = typed.defs[&DefId(Intern::from_ref("converted"))].body else {
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
    let typed = check("Tiny is in 0 to 15\nsource() is < limit: 0\nconverted: source() as Tiny");
    let codes = flaw_codes(&typed);
    assert!(
        codes
            .iter()
            .any(|code| code == "type-integer-narrowing-unproven"),
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
            "Signed is in -9223372036854775808 to 9223372036854775807\nByte is in 0 to 255\nconvert(value Signed) Byte: {body}\n"
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
            "Signed is in -9223372036854775808 to 9223372036854775807\nByte is in 0 to 255\nconvert(value Signed) Byte: {body}\n"
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
    let typed =
        check("Small is in 0 to 15\nByte is in 0 to 255\nconvert(value Small) Byte: value as Byte");
    assert!(flaw_codes(&typed).is_empty(), "{:?}", flaw_codes(&typed));
}

#[test]
fn range_evidence_is_invalidated_by_reassignment_and_mutable_calls() {
    for action in ["value :: replace(value, 300)", "touch(mut value)"] {
        let source = format!(
            "Word is in 0 to 511\nByte is in 0 to 255\nreplace(previous Word, next Word) Word: next\ntouch(value mut Word): 0\nconvert(input Word) Byte:\n    value Word: input\n    if value is in 0 to 255\n        {action}\n    return value as Byte\nreturn 0\n"
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
    let typed = check(
        "Word is in 0 to 511\nByte is in 0 to 255\ntouch(value mut Word) Ready or Failed: Ready\nconvert(input Word) Byte:\n    value Word: input\nreturn when value is in 0 to 255 and touch(mut value) is Ready then value as Byte else 0\n",
    );
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
    let typed = check(
        "Word is in 0 to 511\nByte is in 0 to 255\ntouch(value mut Word): 0\nconvert(input Word) Byte:\n    value Word: input\n    touch(mut value)\nreturn when value is in 0 to 255 then value as Byte else 0\n",
    );
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
    let typed = check(
        "Word is in 0 to 511\nByte is in 0 to 255\nconvert(value Word) Byte:\n    if value is in 0 to 255\n        moved: value\n    return moved as Byte\nreturn 0\n",
    );
    assert!(
        !flaw_codes(&typed)
            .iter()
            .any(|code| code.starts_with("type-integer-narrowing")),
        "{:?}",
        flaw_codes(&typed)
    );
}

#[test]
fn overloaded_comparison_without_a_proof_does_not_narrow_a_named_integer() {
    let typed = check(
        "Word is in 0 to 511\nByte is in 0 to 255\n#infix('<')\nless(left ref Word, right ref Word) True or False: True\nconvert(value Word) Byte: when value < 256 is True then value as Byte else 0\n",
    );
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
            "Word is in 0 to 511\nByte is in 0 to 255\nconvert(input Word):\n    value Word: input\n    if value is in 0 to 255\n        while value is in {guard}\n            narrowed Byte: value as Byte\n            value :: 300\n        loop\n    return\nreturn\n"
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
