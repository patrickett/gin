mod support;
use typecheck::TypedExprKind;

#[test]
fn terminal_parameters_do_not_insert_release_calls() {
    let typed = support::check(&format!(
        "\n{}Leaf has\n    value Word\n    release(self):\n        return\nBox has\n    leaf Leaf\n    release(self) Leaf: return self.leaf\nconsume(value Box):\n    return\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-terminal-value-not-consumed"),
        "{:?}",
        typed.all_flaws()
    );
    assert!(!typed.exprs.kind.iter().any(|kind| matches!(kind,
        TypedExprKind::FnCall { target, args: Some(_), .. }
            if target.0.as_str().ends_with(".release")
    )));
}

#[test]
fn release_trait_requires_an_explicit_implementation() {
    let typed = support::check(&format!(
        "Release has\n    release(self)\n{}Box has Release\n    value Word\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-missing-trait-method")
    );
}

#[test]
fn empty_terminal_default_is_rejected_without_inserting_field_releases() {
    let typed = support::check(&format!(
        "\nRelease has\n    inspect(self ref) Word\n    release(self):\n        return\n{}Leaf has Release\n    value Word\nBox has Release\n    leaf Leaf\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-empty-terminal-default")
    );
    assert!(!typed.exprs.kind.iter().any(|kind| matches!(kind,
        TypedExprKind::FnCall { target, args: Some(_), .. }
            if target.0.as_str().ends_with(".release")
    )));
}

#[test]
fn owned_parameter_still_requires_explicit_consumption() {
    for (body, invalid) in [("", true), ("value.release", false)] {
        let typed = support::check(&format!(
            "{}Leaf has\n    value Word\n    release(self):\n        return\nconsume(value own Leaf):\n    {body}\n    return\n",
            test_fixtures::preludes::WORD_PRELUDE
        ));
        let flaws = support::flaw_codes(&typed);
        if invalid {
            assert!(
                flaws
                    .iter()
                    .any(|code| code == "type-owned-param-not-consumed"),
                "{flaws:?}"
            );
        } else {
            assert!(flaws.is_empty(), "{flaws:?}");
        }
    }
}

#[test]
fn partial_moves_preserve_siblings_and_reject_reuse() {
    let prefix = format!(
        "\n{}Leaf has\n    value Word\n    release(self):\n        return\nPair has\n    first Leaf\n    second Leaf\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    for (body, invalid) in [
        ("self.first.release\n        self.second.release", false),
        ("self.first.release\n        self.first.release", true),
        (
            "self.first.release\n        Pair(first: first, second: second) := self",
            true,
        ),
    ] {
        let typed = support::check(&format!(
            "{prefix}\nPair has\n    release(self):\n        {body}\n        return\nconsume(value Pair):\n    value.release\n    return\n"
        ));
        let flaws = support::flaw_codes(&typed);
        if invalid {
            assert!(
                flaws.iter().any(|code| code.contains("moved")),
                "{body}: {flaws:?}"
            );
        } else {
            assert!(flaws.is_empty(), "{body}: {flaws:?}");
        }
    }
}

#[test]
fn explicit_generic_release_methods_are_specialized() {
    let typed = support::check(&format!(
        "\nRelease has\n    release(self)\n{}Leaf has Release\n    value Word\n    release(self):\n        return\nContainer(element) has Release\n    value element\n    release(self):\n        self.value.release\n        return\nBox has Release\n    value Container(Leaf)\n    release(self):\n        self.value.release\n        return\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.exprs.kind.iter().any(|kind| matches!(kind, TypedExprKind::FnCall { target, args: Some(_), .. } if target.0.as_str().starts_with("Container.release$specialized$"))));
}

#[test]
fn raw_pointer_store_transfers_ownership_of_the_value() {
    for (after, invalid) in [("", false), ("value.release", true)] {
        let typed = support::check(&format!(
            "\n{}Leaf has\n    value Word\n    release(self):\n        return\nstore(destination @Leaf, value own Leaf):\n    pointer: destination\n    pointer.[0]:: value\n    {after}\n    return\n",
            test_fixtures::preludes::WORD_PRELUDE
        ));
        let flaws = support::flaw_codes(&typed);
        if invalid {
            assert!(flaws.iter().any(|code| code.contains("moved")), "{flaws:?}");
        } else {
            assert!(flaws.is_empty(), "{flaws:?}");
        }
    }
}

#[test]
fn explicit_field_release_does_not_schedule_parent_release() {
    let typed = support::check(&format!(
        "\n{}Leaf has\n    value Word\n    release(self):\n        return\nOwner has\n    leaf Leaf\n    release(self):\n        self.leaf.release\n        return\nconsume(value Owner):\n    value.leaf.release\n    return\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(!typed.exprs.kind.iter().any(|kind| matches!(kind,
        TypedExprKind::FnCall { target, args: Some(_), .. }
            if target.0.as_str() == "Owner.release"
    )));
}

#[test]
fn arrays_do_not_require_automatic_cleanup() {
    for body in [
        "consume(values [Leaf; 4097]):\n    return\n",
        "consume(values [Leaf; 2], index is in 0 to 1):\n    Leaf.release(values.[index])\n    return\n",
    ] {
        let typed = support::check(&format!(
            "{}Leaf has\n    value Word\n    release(self):\n        return\n{body}",
            test_fixtures::preludes::WORD_PRELUDE
        ));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    }
}

#[test]
fn released_array_elements_cannot_be_used_twice() {
    let typed = support::check(&format!(
        "\n{}Leaf has\n    value Word\n    release(self):\n        return\nconsume(values [Leaf; 2]):\n    Leaf.release(values.[0])\n    Leaf.release(values.[0])\n    return\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code.contains("moved")),
        "{:?}",
        typed.all_flaws()
    );
}
