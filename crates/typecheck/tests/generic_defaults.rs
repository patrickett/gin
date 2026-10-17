mod support;

use internment::Intern;
use parser::cursor::TokenCursor;
use support::check;
use typecheck::TagId;

#[test]
fn nested_generic_constructor_uses_declared_return_arguments() {
    let source = format!(
        "{}Packet(element, count BigInt) has\n    values Array(element, count)\n    finish(self):\n        return\nEnvelope(payload) has\n    value payload\n    finish(self):\n        return\nmake() Envelope(Packet(Word, 2)):\n    return Envelope(value: Packet(values: [1, 2]))\n#fixed_array(element, count)\nArray(element, count BigInt) has\n    finish(self):\n        return\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let parsed = TokenCursor::parse_source(&source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );

    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let bind = &typed.defs[&typecheck::DefId::named(Intern::from_ref("make"))];
    let definition = typed
        .type_registry
        .resolved_definition_for_type(&bind.return_type);
    let typecheck::ty::Ty::Record { fields, .. } = definition else {
        panic!("expected envelope: {definition:?}");
    };
    let packet = typed
        .type_registry
        .resolved_definition_for_type(&fields[0].1);
    let typecheck::ty::Ty::Record { fields, .. } = packet else {
        panic!("expected packet: {packet:?}");
    };
    let (element, length) =
        typecheck::representation::Repr::array_parts(&fields[0].1, Some(&typed.type_registry))
            .expect("specialized array field");
    assert_eq!(length, 2);
    assert_eq!(element.type_name().unwrap().as_str(), "Word");
}

#[test]
fn omitted_generic_type_default_resolves_to_the_declared_default() {
    let typed = check(
        "Int is in 0 to 255\nInt has\n    finish(self):\n        return\nGlobalAllocator has\n    marker Int\n    finish(self):\n        return\nArenaAllocator has\n    marker Int\n    finish(self):\n        return\nVec(element, allocator: GlobalAllocator) has\n    value element\n    finish(self):\n        return\nHolder has\n    value Vec(Int)\n    finish(self):\n        self.value.finish\n        return\nExplicit has\n    value Vec(Int, allocator: ArenaAllocator)\n    finish(self):\n        self.value.finish\n        return\nReordered has\n    value Vec(allocator: ArenaAllocator, element: Int)\n    finish(self):\n        self.value.finish\n        return\n",
    );

    assert!(
        typed.all_flaws().is_empty(),
        "defaulted generic application should typecheck: {:?}",
        typed.all_flaws()
    );

    let allocator_for = |owner_name: &str| {
        let owner = typed
            .tags
            .get(&TagId(Intern::new(owner_name.to_string())))
            .expect("owner declaration");
        let typecheck::ty::Ty::Record { fields, .. } = typed
            .type_registry
            .resolved_definition_for_type(&owner.resolved_ty)
        else {
            panic!("owner should resolve to a record: {:?}", owner.resolved_ty);
        };
        let (_, value_ty) = fields
            .iter()
            .find(|(name, _)| name.as_str() == "value")
            .expect("owner.value field");
        let typecheck::ty::Ty::Named { instance, .. } = value_ty.as_ref() else {
            panic!("owner.value should be a named Vec: {value_ty:?}");
        };

        assert_eq!(instance.arguments.len(), 2);
        assert_eq!(instance.arguments[1].0.as_str(), "allocator");
        let ast::TyArg::Type(allocator) = &instance.arguments[1].1 else {
            panic!("Vec allocator argument should be a type");
        };
        let typecheck::ty::Ty::Named { name, .. } = allocator.as_ref() else {
            panic!("Vec allocator should be named: {allocator:?}");
        };
        name.as_str().to_owned()
    };

    assert_eq!(allocator_for("Holder"), "GlobalAllocator");
    assert_eq!(allocator_for("Explicit"), "ArenaAllocator");
    assert_eq!(allocator_for("Reordered"), "ArenaAllocator");
}

#[test]
fn bare_generic_type_uses_defaults_in_fields_and_function_signatures() {
    let typed = check(&format!(
        "{}DefaultStore has\n    marker Word\n    finish(self):\n        return\nText(store: DefaultStore) has\n    value store\n    finish(self):\n        self.value.finish\n        return\nHolder has\n    text Text\n    finish(self):\n        self.text.finish\n        return\npass(value own Text) Text: return value\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn named_arguments_do_not_skip_or_overwrite_other_defaults() {
    use ast::{Ty, TyArg};
    use parser::query::SourceParseExt;
    for (argument, first, second) in [
        ("second: Wide", "Byte", "Wide"),
        ("first: Wide", "Wide", "Byte"),
    ] {
        let source = format!(
            "{}Wide is in 0 to 65535\nWide has\n    finish(self):\n        return\nPair(first: Byte, second: Byte) has\n    left first\n    right second\n    finish(self):\n        self.left.finish\n        self.right.finish\n        return\nHolder has\n    pair Pair({argument})\n    finish(self):\n        self.pair.finish\n        return\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let typed = check(&source);
        let owner = &typed.tags[&TagId(Intern::from_ref("Holder"))];
        let Ty::Record { fields, .. } = typed
            .type_registry
            .resolved_definition_for_type(&owner.resolved_ty)
        else {
            panic!("expected holder record")
        };
        let Ty::Named { instance, .. } = fields[0].1.as_ref() else {
            panic!("expected nominal pair")
        };
        assert_eq!(
            instance.arguments,
            vec![
                (
                    Intern::from_ref("first"),
                    TyArg::Type(Box::new(
                        typed.tags[&TagId(Intern::from_ref(first))]
                            .resolved_ty
                            .clone()
                    ))
                ),
                (
                    Intern::from_ref("second"),
                    TyArg::Type(Box::new(
                        typed.tags[&TagId(Intern::from_ref(second))]
                            .resolved_ty
                            .clone()
                    ))
                ),
            ],
            "{argument}"
        );
        assert!(
            typed.all_flaws().is_empty(),
            "{argument}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn invalid_named_generic_arguments_are_diagnosed_in_type_annotations() {
    use parser::query::SourceParseExt;
    for (arguments, code) in [
        ("unknown: Byte", "type-unknown-generic-argument"),
        (
            "first: Byte, first: Byte",
            "type-duplicate-generic-argument",
        ),
        ("Byte, first: Byte", "type-duplicate-generic-argument"),
    ] {
        for annotation in [
            format!(
                "Holder has\n    pair Pair({arguments})\n    finish(self):\n        self.pair.finish\n        return\n"
            ),
            format!("Alias is Pair({arguments})\n"),
            format!("pass(value own Pair({arguments})) Pair: return value\n"),
            format!("pass(value own Pair) Pair({arguments}): return value\n"),
            format!(
                "Holder has\n    pair Pair(first: Pair({arguments}))\n    finish(self):\n        self.pair.finish\n        return\n"
            ),
        ] {
            let source = format!(
                "{}Pair(first: Byte, second: Byte) has\n    left first\n    right second\n    finish(self):\n        self.left.finish\n        self.right.finish\n        return\n{annotation}",
                test_fixtures::preludes::BYTE_PRELUDE
            );
            let parsed = source.parse_source_full();
            assert!(
                parsed.symptoms.is_empty(),
                "{annotation}: {:?}",
                parsed.symptoms
            );
            let typed = check(&source);
            assert!(
                typed
                    .all_flaws()
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == code),
                "{annotation}: {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn generic_argument_checks_cover_local_and_callable_type_surfaces() {
    use parser::query::SourceParseExt;
    for annotation in [
        "pass(value own Pair) Pair:\n    local Pair(unknown: Byte): value\n    return local\n",
        "pass(value own Pair) result Pair(unknown: Byte): return value\n",
        "Pair(unknown: Byte).read(self ref) Byte: return self.left\n",
    ] {
        let source = format!(
            "{}Pair(first: Byte, second: Byte) has\n    left first\n    right second\n    finish(self):\n        self.left.finish\n        self.right.finish\n        return\n{annotation}",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{annotation}: {:?}",
            parsed.symptoms
        );
        let typed = check(&source);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-unknown-generic-argument"),
            "{annotation}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn compound_type_surfaces_validate_nested_generic_arguments() {
    use parser::query::SourceParseExt;
    for (arguments, code) in [
        ("first: Byte", None),
        ("unknown: Byte", Some("type-unknown-generic-argument")),
        (
            "first: Byte, first: Byte",
            Some("type-duplicate-generic-argument"),
        ),
    ] {
        for surface in [
            format!("[Pair({arguments}); 2]"),
            format!("[Byte, Pair({arguments})]"),
        ] {
            let source = format!(
                "{}Pair(first: Byte, second: Byte) has\n    left first\n    right second\n    finish(self):\n        self.left.finish\n        self.right.finish\n        return\npass(value ref {surface}):\n    return\n",
                test_fixtures::preludes::BYTE_PRELUDE
            );
            let parsed = source.parse_source_full();
            assert!(
                parsed.symptoms.is_empty(),
                "{surface}: {:?}",
                parsed.symptoms
            );
            let typed = check(&source);
            if let Some(code) = code {
                assert!(
                    typed
                        .all_flaws()
                        .iter()
                        .any(|(_, flaw)| flaw.code.slug() == code),
                    "{surface}: {:?}",
                    typed.all_flaws()
                );
            } else {
                assert!(
                    typed.all_flaws().is_empty(),
                    "{surface}: {:?}",
                    typed.all_flaws()
                );
            }
        }
    }
}

#[test]
fn generic_declaration_and_union_surfaces_validate_argument_names() {
    use parser::query::SourceParseExt;
    for (arguments, expected) in [
        ("first: Byte", None),
        ("unknown: Byte", Some("type-unknown-generic-argument")),
        (
            "first: Byte, first: Byte",
            Some("type-duplicate-generic-argument"),
        ),
    ] {
        for declaration in [
            format!(
                "Holder(element: Pair({arguments})) has\n    value element\n    finish(self):\n        self.value.finish\n        return\n"
            ),
            format!(
                "Choice is Wrapped(value Pair({arguments})) or Empty\nChoice.finish(self): return when self is Wrapped(value) then value.finish else idle\nidle:\n    return\n"
            ),
            format!(
                "Choice is Pair({arguments}) or Empty\nChoice.finish(self): return when self is Pair(value) then value.finish else idle\nidle:\n    return\n"
            ),
        ] {
            let source = format!(
                "{}Pair(first: Byte, second: Byte) has\n    left first\n    right second\n    finish(self):\n        self.left.finish\n        self.right.finish\n        return\n{declaration}",
                test_fixtures::preludes::BYTE_PRELUDE
            );
            let parsed = source.parse_source_full();
            assert!(
                parsed.symptoms.is_empty(),
                "{declaration}: {:?}",
                parsed.symptoms
            );
            let typed = check(&source);
            if let Some(code) = expected {
                assert!(
                    typed
                        .all_flaws()
                        .iter()
                        .any(|(_, flaw)| flaw.code.slug() == code),
                    "{declaration}: {:?}",
                    typed.all_flaws()
                );
            } else {
                assert!(
                    typed.all_flaws().is_empty(),
                    "{declaration}: {:?}",
                    typed.all_flaws()
                );
            }
        }
    }
}

#[test]
fn extra_generic_arguments_are_rejected_before_positional_mapping() {
    use parser::query::SourceParseExt;
    for declaration in [
        "Holder(element: Pair(Byte, Byte, Byte)) has\n    value element\n    finish(self):\n        self.value.finish\n        return\n",
        "pass(value own Pair) Pair:\n    local Pair(Byte, Byte, Byte): value\n    return local\n",
        "Choice is Pair(Byte, Byte, Byte) or Empty\nChoice.finish(self): return when self is Pair(value) then value.finish else idle\nidle:\n    return\n",
    ] {
        let source = format!(
            "{}Pair(first: Byte, second: Byte) has\n    left first\n    right second\n    finish(self):\n        self.left.finish\n        self.right.finish\n        return\n{declaration}",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{declaration}: {:?}",
            parsed.symptoms
        );
        let typed = check(&source);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-excess-generic-arguments"),
            "{declaration}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn types_without_parameters_reject_type_arguments() {
    use parser::query::SourceParseExt;
    for declaration in [
        "Alias is Byte(Byte)\n",
        "pass(value ref Byte(Byte)):\n    return\n",
        "Holder(element: Byte(Byte)) has\n    value element\n    finish(self):\n        self.value.finish\n        return\n",
        "Choice is Byte(Byte) or Empty\nChoice.finish(self): return when self is Byte(value) then value.finish else idle\nidle:\n    return\n",
    ] {
        let source = format!("{}{declaration}", test_fixtures::preludes::BYTE_PRELUDE);
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{declaration}: {:?}",
            parsed.symptoms
        );
        let typed = check(&source);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-excess-generic-arguments"),
            "{declaration}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn zero_parameter_types_keep_value_constructors_and_new_union_payloads() {
    use parser::query::SourceParseExt;
    let source = format!(
        "{}Alias is Byte\nBox has\n    value Byte\n    finish(self):\n        self.value.finish\n        return\nChoice is Wrapped(value Byte) or Empty\nChoice.finish(self): return when self is Wrapped(value) then value.finish else idle\nidle:\n    return\nrun:\n    box: Box(value: 7 as Byte)\n    box.finish\n    choice: Wrapped(7 as Byte)\n    choice.finish\n    return\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn union_result_annotations_validate_generic_arguments() {
    use parser::query::SourceParseExt;
    for (arguments, expected) in [
        ("first: Byte", None),
        ("unknown: Byte", Some("type-unknown-generic-argument")),
        (
            "first: Byte, first: Byte",
            Some("type-duplicate-generic-argument"),
        ),
        ("Byte, Byte, Byte", Some("type-excess-generic-arguments")),
    ] {
        for variants in [
            format!("First -> Pair({arguments}) or Last"),
            format!("First or Last -> Pair({arguments})"),
        ] {
            let source = format!(
                "{}Pair(first: Byte, second: Byte) has\n    left first\n    right second\n    finish(self):\n        self.left.finish\n        self.right.finish\n        return\nChoice is {variants}\nChoice.finish(self):\n    return\n",
                test_fixtures::preludes::BYTE_PRELUDE
            );
            let parsed = source.parse_source_full();
            assert!(
                parsed.symptoms.is_empty(),
                "{variants}: {:?}",
                parsed.symptoms
            );
            let typed = check(&source);
            if let Some(code) = expected {
                assert!(
                    typed
                        .all_flaws()
                        .iter()
                        .any(|(_, flaw)| flaw.code.slug() == code),
                    "{variants}: {:?}",
                    typed.all_flaws()
                );
            } else {
                assert!(
                    typed.all_flaws().is_empty(),
                    "{variants}: {:?}",
                    typed.all_flaws()
                );
            }
        }
    }
}
