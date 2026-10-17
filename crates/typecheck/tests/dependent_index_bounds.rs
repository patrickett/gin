mod support;

use internment::Intern;
use parser::query::SourceParseExt;
use support::{check_with, expect_flaws_with_named, package};

const LOOKUP: &str = include_str!("../../test-fixtures/fixtures/runtime_bounded_lookup.gin");
const PRELUDE: &str = concat!(
    include_str!("../../test-fixtures/fixtures/runtime_bounded_lookup.gin"),
    "\n"
);

const CASES: &[(&str, &str, &[&str])] = &[
    (
        "runtime_guard_satisfies_a_separate_lookup_contract",
        "read(view ref View(Byte), index Count) Byte:\n    when index.as_num < view.length.as_num then\n        return at(view, index)\n    return 0\n",
        &[],
    ),
    (
        "unvalidated_index_cannot_call_the_proven_lookup",
        "read(view ref View(Byte), index Count) Byte: return at(view, index)\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "another_views_length_cannot_authorize_the_lookup",
        "read(view ref View(Byte), other ref View(Byte), index Count) Byte:\n    when index.as_num < other.length.as_num then\n        return at(view, index)\n    return 0\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "an_unproved_narrowing_is_rejected_before_it_can_authorize_a_lookup",
        "Nibble is in 0 to 15\nNibble has Numeric\n    release(self):\n        return\nread(view ref View(Byte), index Count) Byte:\n    narrowed: index.as_num as Nibble\n    when narrowed.as_num < view.length.as_num then\n        return at(view, index)\n    return 0\n",
        &["type-integer-narrowing-failed"],
    ),
    (
        "changing_a_checked_index_requires_a_new_proof",
        "read(view ref View(Byte), index Count, next Count) Byte:\n    position: index\n    when position.as_num >= view.length.as_num then\n        return 0\n    position :: Count.replace(position, next)\n    return at(view, position)\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "changing_a_checked_index_to_a_known_invalid_value_is_rejected",
        "read(view ref View(Byte), index Count) Byte:\n    position: index\n    when position.as_num >= view.length.as_num then\n        return 0\n    position :: Count.replace(position, 255)\n    return at(view, position)\n",
        &["type-parameter-refinement-failed"],
    ),
];

#[test]
fn dependent_index_bound_cases() {
    for &(name, body, expected) in CASES {
        expect_flaws_with_named(PRELUDE, body, expected, name);
    }
}

#[test]
fn bounds_evidence_matches_the_current_access_and_its_operands() {
    use internment::Intern;
    use typecheck::{BindBody, DefId, TypedExprKind};

    let mut typed = check_with(PRELUDE, "");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let BindBody::Expr(access) = typed.defs[&DefId::named(Intern::from_ref("at"))].body else {
        panic!("expected a direct accessor");
    };
    let TypedExprKind::BufGet { buf, index } = typed.exprs.kind[access.index()] else {
        panic!("expected an indexed read");
    };
    let program = typed.resolved_program();
    assert!(program.sequence_index_is_proven(access, buf, index));
    assert!(!program.sequence_index_is_proven(access, index, buf));
    assert!(!program.sequence_index_is_proven(buf, buf, index));

    typed.exprs.kind[access.index()] = TypedExprKind::BufGet {
        buf: index,
        index: buf,
    };
    let program = typed.resolved_program();
    assert!(!program.sequence_index_is_proven(access, buf, index));
    assert!(!program.sequence_index_is_proven(access, index, buf));

    typed.exprs.kind[access.index()] = TypedExprKind::BufGet { buf, index };
    assert!(
        typed
            .resolved_program()
            .sequence_index_is_proven(access, buf, index)
    );
}

#[test]
fn bounds_contracts_are_checked_across_module_boundaries() {
    for guarded in [false, true] {
        let body = if guarded {
            "    when index.as_num < view.length.as_num then\n        return at(view, index)\n    return 0\n"
        } else {
            "    return at(view, index)\n"
        };
        let client = format!(
            "use sample.model Byte, Count, View, at\nread(view ref View(Byte), index Count) Byte:\n{body}"
        );
        let output = package("sample", &[("model", LOOKUP), ("client", &client)]);
        let codes: Vec<_> = output.iter().flat_map(support::flaw_codes).collect();
        let expected = if guarded {
            vec![]
        } else {
            vec!["type-parameter-refinement-unproven"]
        };
        assert_eq!(codes, expected);
        let model = &output[0];
        let at = model
            .defs
            .get(&typecheck::DefId::named(Intern::from_ref("at")))
            .unwrap_or_else(|| panic!("accessor missing: {:?}", model.defs.keys()));
        let typecheck::BindBody::Expr(access) = at.body else {
            panic!("expected a direct accessor");
        };
        let typecheck::TypedExprKind::BufGet { buf, index } = model.exprs.kind[access.index()]
        else {
            panic!("expected an indexed read");
        };
        assert!(
            model
                .resolved_program()
                .sequence_index_is_proven(access, buf, index)
        );
    }
}

#[test]
fn editing_a_contract_updates_cached_bounds_evidence() {
    use ast::ty::PackageInstanceKey;
    use internment::Intern;
    use typecheck::transform::{
        BodyReuse, PackageCheckSession, PackageTransformOptions,
        transform_package_with_shared_context_and_session,
    };

    let mut session =
        PackageCheckSession::new(1.try_into().unwrap(), BodyReuse::ExactInputs).unwrap();
    for expected in [true, true, false, false, true] {
        let source = if expected {
            LOOKUP.to_owned()
        } else {
            LOOKUP.replace("index Count and < view.length", "index Count")
        };
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let output = transform_package_with_shared_context_and_session(
            vec![parsed.ast],
            PackageTransformOptions::FULL,
            PackageInstanceKey::workspace("sample", "1"),
            &mut session,
        );
        let typed = &output.typed_asts[0];
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let at = &typed.defs[&typecheck::DefId::named(Intern::from_ref("at"))];
        let typecheck::BindBody::Expr(access) = at.body else {
            panic!("expected a direct accessor");
        };
        let typecheck::TypedExprKind::BufGet { buf, index } = typed.exprs.kind[access.index()]
        else {
            panic!("expected an indexed read");
        };
        assert_eq!(
            typed
                .resolved_program()
                .sequence_index_is_proven(access, buf, index),
            expected,
        );
    }
}
