mod support;

use support::check_unflawed_with;

const TYPES: &str = r#"Word is in 0 to 255
Word has
    release(self):
        return
Counter has
    increment(self mut)
    release(self)
Store has Counter
    value Word
    increment(self mut):
        return
    release(self):
        return
"#;

#[test]
fn mutable_constrained_parameter_infers_and_preserves_concrete_value() {
    let typed = check_unflawed_with(
        TYPES,
        "bump(a mut has Counter): return a.increment\nrun:\n    store: Store(value: 1 as Word)\n    bump(store)\n    store.release\n    return\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.defs.values().any(|bind| {
        bind.name.as_str().starts_with("bump$specialized$")
            && bind.params.len() == 1
            && !bind.requires_specialization()
    }));
}

#[test]
fn mutable_unconstrained_parameter_is_an_inferred_value() {
    let typed = check_unflawed_with(
        TYPES,
        "bump(a mut): return a.increment\nrun:\n    store: Store(value: 1 as Word)\n    bump(store)\n    store.release\n    return\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn matching_methods_without_declared_trait_do_not_satisfy_constraint() {
    let typed = check_unflawed_with(
        TYPES,
        "Impostor has\n    increment(self mut):\n        return\n    release(self):\n        return\nbump(a mut has Counter): return a.increment\nrun:\n    store: Impostor()\n    bump(store)\n    store.release\n    return\n",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-parameter-trait-constraint"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn terminal_constrained_parameter_is_a_value() {
    let typed = check_unflawed_with(
        TYPES,
        "consume(a has Counter): return a.release\nrun: consume(Store(value: 1 as Word))\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn shared_argument_cannot_satisfy_mutable_parameter() {
    let typed = check_unflawed_with(
        TYPES,
        "bump(a mut has Counter): return a.increment\nobserve(a ref Store): return bump(a)\n",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-mutable-borrow-from-shared"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn bare_parameter_is_an_inferred_value_and_remains_in_the_body() {
    let typed = check_unflawed_with(
        TYPES,
        "consume(a): return a.release\nrun: consume(Store(value: 1 as Word))\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn inherited_trait_satisfies_constraint() {
    let typed = check_unflawed_with(
        TYPES,
        "Advanced has Counter\nNested has Advanced\n    increment(self mut):\n        return\n    release(self):\n        return\nbump(a mut has Counter): return a.increment\nrun:\n    store: Nested()\n    bump(store)\n    store.release\n    return\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn generic_trait_arguments_must_match() {
    for (argument, valid) in [("Word", true), ("Store", false)] {
        let typed = check_unflawed_with(
            TYPES,
            &format!(
                "Convert(element) has\n    convert(self ref) element\nConversion has Convert(Word)\n    convert(self ref) Word: return 1 as Word\n    release(self):\n        return\ninspect(a ref has Convert({argument})):\n    return\nrun:\n    converter: Conversion()\n    inspect(converter)\n    converter.release\n    return\n"
            ),
        );
        if valid {
            assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        } else {
            assert!(
                typed
                    .all_flaws()
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "type-parameter-trait-constraint"),
                "{:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn unused_constraints_are_validated_at_declaration() {
    for (source, code) in [
        (
            "inspect(a ref has Missing):\n    return\n",
            "type-unknown-trait",
        ),
        (
            "inspect(a ref has Counter(Word)):\n    return\n",
            "type-trait-argument-count",
        ),
    ] {
        let typed = check_unflawed_with(TYPES, source);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == code),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn separate_bare_parameters_infer_independent_types_with_named_arguments() {
    let typed = check_unflawed_with(
        TYPES,
        "consume(first, second):\n    first.release\n    second.release\n    return\nrun: consume(second: 7 as Word, first: Store(value: 1 as Word))\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn generic_constraint_substitutes_a_shared_type_parameter() {
    let typed = check_unflawed_with(
        TYPES,
        "Convert(element) has\n    convert(self ref) element\nConversion has Convert(Word)\n    convert(self ref) Word: return 1 as Word\n    release(self):\n        return\ninspect(a ref has Convert(x), value x): return value.release\nrun:\n    converter: Conversion()\n    inspect(converter, 1 as Word)\n    converter.release\n    return\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn signature_preserves_the_constraint_and_borrowing_convention() {
    let typed = check_unflawed_with(TYPES, "inspect(a ref has Counter):\n    return\n");
    let bind = &typed.defs[&typecheck::DefId::named(internment::Intern::from_ref("inspect"))];
    assert_eq!(&*bind.signature_surface, "inspect(a ref has Counter)");
}

#[test]
fn bare_identity_preserves_scalar_array_and_sequence_values() {
    for value in ["1 as Word", "123", "'abc'", "[1 as Word, 2 as Word]"] {
        let typed = check_unflawed_with(
            TYPES,
            &format!("identity(value): return value\nresult: identity({value})\n"),
        );
        assert!(
            typed.all_flaws().is_empty(),
            "{value}: {:?}",
            typed.all_flaws()
        );
        let specialized = typed
            .defs
            .values()
            .find(|bind| bind.name.as_str().starts_with("identity$specialized$"))
            .expect("specialized identity");
        assert!(!specialized.requires_specialization(), "{value}");
        assert_eq!(specialized.params[0].1, specialized.return_type, "{value}");
    }
}

#[test]
fn implementations_cannot_change_parameter_trait_contracts() {
    let typed = check_unflawed_with(
        TYPES,
        "Other has release(self)\nUsesCounter has\n    accept(self ref, a mut has Counter)\nImplementation has UsesCounter\n    accept(self ref, a mut has Other):\n        return\n    release(self):\n        return\n",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-trait-method-mismatch"),
        "{:?}",
        typed.all_flaws()
    );
}
