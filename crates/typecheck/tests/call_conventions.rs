mod support;

use support::{check_parsed, flaw_codes};

#[test]
fn declarations_borrow_direct_named_and_receiver_arguments() {
    let typed = check_parsed(&format!(
        "{}replace(previous Byte, next Byte) Byte: return next\nBox has\n    finish(self):\n        return\n    value Byte\n    read(self ref) Byte: return self.value\n    write(self mut, value Byte): return self.value:: replace(self.value, value)\nread(value ref Box) Byte: return value.value\nwrite(value mut Box, replacement Byte): return value.value:: replace(value.value, replacement)\nrun:\n    box: Box(value: 1)\n    read(box)\n    read(box)\n    write(replacement: 2, value: box)\n    box.write(3)\n    return box.read\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn borrowed_arguments_are_reborrowed() {
    let typed = check_parsed(&format!(
        "{}read(value ref Byte) Byte: return deref value\nwrite(value mut Byte):\n    value:: 7\n    return\nforward(value mut Byte) Byte:\n    read(value)\n    write(value)\n    return read(value)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn owned_arguments_still_move() {
    let typed = check_parsed(&format!(
        "{}Box has\n    finish(self):\n        return\n    value Byte\ntake(value own Box) Box: return value\nrun:\n    box: Box(value: 1)\n    take(box)\n    return take(box)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-use-after-move"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn shared_arguments_cannot_supply_mutable_parameters() {
    let typed = check_parsed(&format!(
        "{}write(value mut Byte):\n    value:: 7\n    return\nforward(value ref Byte): return write(value)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-mutable-borrow-from-shared"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn overloads_match_borrowed_referent_types() {
    let typed = check_parsed(&format!(
        "{}A has\n    finish(self):\n        return\n    value Byte\nB has\n    finish(self):\n        return\n    value Byte\nread(value ref A) Byte: return value.value\nread(value ref B) Byte: return value.value\nrun:\n    a: A(value: 1)\n    b: B(value: 2)\n    read(a)\n    return read(b)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn mutable_arguments_cannot_borrow_shared_fields_or_immutable_places() {
    for source in [
        format!(
            "{}Box has\n    finish(self):\n        return\n    value Byte\nwrite(value mut Byte):\n    value:: 7\n    return\nforward(value ref Box): return write(value.value)\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}write(value mut Byte):\n    value:: 7\n    return\nrun:\n    value := 1 as Byte\n    write(value)\n    return\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}Box has\n    finish(self):\n        return\n    value Byte\nwrite(value mut Byte):\n    value:: 7\n    return\nrun:\n    value: Box(value: 1)\n    shared: ref value\n    write(shared.value)\n    return\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}write(value mut Byte):\n    value:: 7\n    return\nforward(value ref Byte): return write(deref value)\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
    ] {
        let typed = check_parsed(&source);
        assert!(
            flaw_codes(&typed).iter().any(|code| matches!(
                code.as_str(),
                "type-mutable-borrow-from-immutable" | "type-mutable-borrow-from-shared"
            )),
            "unexpectedly accepted {source}"
        );
    }
}

#[test]
fn calls_reject_overlapping_mutated_arguments() {
    let typed = check_parsed(&format!(
        "{}write(value mut{{a}} Byte, source ref{{b}} Byte): return value:: deref source\nrun:\n    value: 1 as Byte\n    write(value, value)\n    return\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-overlapping-target-group-arguments"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn borrowing_does_not_choose_between_convention_only_overloads() {
    let typed = check_parsed(&format!(
        "{}read(value ref Byte) Byte: return deref value\nread(value mut Byte) Byte: return deref value\nrun:\n    value: 1 as Byte\n    return read(value)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-ambiguous-overload"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn borrowing_temporaries_requires_a_local_binding() {
    let typed = check_parsed(&format!(
        "{}read(value ref Byte) Byte: return deref value\nrun: read(1)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-reference-to-temporary"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn reference_fields_borrow_constructor_arguments() {
    let typed = check_parsed(&format!(
        "{}View has\n    finish(self):\n        return\n    value ref Byte\nread(value ref Byte) Byte: return deref value\nrun:\n    value: 7 as Byte\n    view: View(value)\n    return read(view.value)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn anonymous_integer_parameters_preserve_reference_conventions() {
    let typed = check_parsed(
        "read(value ref is in 0 to 255) is in 0 to 255: return value + 0\nwrite(value mut is in 0 to 255):\n    value:: 7\n    return\nforward(value mut is in 0 to 255) is in 0 to 255:\n    read(value)\n    write(value)\n    return read(value)\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    for bind in typed.defs.values() {
        assert!(
            bind.params.iter().all(|(_, ty)| ty.is_ref()),
            "{:?}",
            bind.params
        );
    }
}
