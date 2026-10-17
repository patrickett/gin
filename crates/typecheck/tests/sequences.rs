mod support;
use ast::Ty;
use support::{body, check, flaw_codes};

#[test]
fn sequence_shapes_preserve_optional_names_and_element_types() {
    for source in [
        "values: 10, 20\n",
        "values: [10, 20]\n",
        "values: [x: 10, y: 20]\n",
        "values: x: 10, 20\n",
        "Flag is On or Off\nvalues: [10, On]\n",
        "values: [10, 20], [30, 40]\n",
    ] {
        let typed = check(source);
        assert!(
            flaw_codes(&typed).is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
        let ty = &typed.exprs.ty[body(&typed, "values").as_usize()];
        assert!(
            matches!(ty, Ty::Array { .. } | Ty::Sequence(_)),
            "{source}: {ty:?}"
        );
        assert!(
            typecheck::representation::Repr::derive(ty, Some(&typed.type_registry)).is_ok(),
            "{source}: {ty:?}"
        );
    }
}

#[test]
fn named_single_result_retains_its_projection() {
    let typed = check(&format!(
        "{}get() value Byte: return 42\nmain: get().value\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
    let ty = &typed.defs[&typecheck::DefId::named(internment::Intern::from_ref("get"))].return_type;
    assert!(
        matches!(ty, Ty::Sequence(fields) if fields.len() == 1 && fields[0].name.unwrap().as_str() == "value"),
        "{ty:?}"
    );
}

#[test]
fn return_sequences_support_named_unnamed_mixed_and_nested_results() {
    for source in [
        format!(
            "{}Factory has\n    get() value Byte: return 42\nmain: Factory.get().value\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}Factory has\n    get() first Byte, Byte: return 1, 2\nmain: Factory.get().1\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}get() [Byte; 2]: return 1, 2\nmain: get().0\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}get() first Byte, second Byte: return 1, 2\nmain: get().second\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}get() Byte, Byte: return 1, 2\nmain: get().0\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}get() first Byte, Byte: return 1, 2\nmain: get().1\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}get() first Byte, second Byte: return [first: 1, second: 2]\nmain: get().first\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}get() values [Byte; 2], count Byte: return [1, 2], 2\nmain: get().values.0\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
    ] {
        let typed = check(&source);
        assert!(
            flaw_codes(&typed).is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn sequence_context_checks_names_types_arity_and_indices() {
    for (source, code) in [
        (
            format!(
                "{}get() first Byte, second Byte: return [second: 1, first: 2]\n",
                test_fixtures::preludes::BYTE_PRELUDE
            ),
            "type-sequence-name-mismatch",
        ),
        (
            format!(
                "{}get() first Byte, second Byte: return [1]\n",
                test_fixtures::preludes::BYTE_PRELUDE
            ),
            "type-sequence-arity",
        ),
        (
            format!(
                "{}Flag is On or Off\nget() first Byte, second Byte: return [1, On]\n",
                test_fixtures::preludes::BYTE_PRELUDE
            ),
            "type-sequence-element-mismatch",
        ),
        (
            "values: x: 1, y: 2\nmain: values.2\n".to_string(),
            "type-sequence-index-out-of-bounds",
        ),
        (
            "values: x: 1, y: 2\nmain: values.missing\n".to_string(),
            "type-unknown-field",
        ),
        (
            "Index is in 0 to 1\nFlag is On or Off\nmain(index Index): return [1, On].[index]\n"
                .to_string(),
            "type-sequence-dynamic-index",
        ),
    ] {
        let typed = check(&source);
        assert!(
            flaw_codes(&typed).iter().any(|found| found == code),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn named_sequences_can_be_passed_stored_and_forwarded() {
    let typed = check(&format!(
        "{}get() first Byte, second Byte: return 1, 2\nidentity(value [first Byte, second Byte]) [first Byte, second Byte]: return value\nforward() first Byte, second Byte: return identity(get())\nmain:\n    saved: forward()\n    return saved.second\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn explicit_singleton_and_empty_sequences_remain_sequences() {
    for (source, length) in [("values: [1]\n", 1), ("values: []\n", 0)] {
        let typed = check(source);
        assert!(
            flaw_codes(&typed).is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
        match &typed.exprs.ty[body(&typed, "values").as_usize()] {
            Ty::Sequence(fields) => assert_eq!(fields.len(), length),
            Ty::Array { size, .. } => assert_eq!(size.as_const_int(), Some(length as i128)),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn constant_sequence_projection_preserves_names_positions_and_nesting() {
    for (source, expected) in [
        ("main: [x: 10, y: 20].x\n", 10),
        ("main: [x: 10, y: 20].1\n", 20),
        ("main: [[x: 10, y: 20], [x: 30, y: 40]].1.y\n", 40),
        ("main: [10, 20].0\n", 10),
    ] {
        let typed = check(source);
        assert!(
            flaw_codes(&typed).is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
        let value = typecheck::staging::normalize(
            &typed,
            body(&typed, "main"),
            &typecheck::subst::DepSubst::new(),
        );
        assert!(
            matches!(value, typecheck::staging::NormalizeResult::Value(ast::ConstValue::Int(value)) if value == i256::I256::from(expected)),
            "{source}: {value:?}"
        );
    }
}

#[test]
fn named_single_reference_results_preserve_their_target_group() {
    let typed = check(&format!(
        "{}get(value ref{{a}} Byte) selected ref{{a}} Byte: return value\nforward(value ref{{a}} Byte) selected ref{{a}} Byte: return get(value)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
}
