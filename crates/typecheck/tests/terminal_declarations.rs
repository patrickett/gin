mod support;

use diagnostic::{Category, Diagnostic};
use support::package;

fn check(sources: &[(&str, &str)]) -> Vec<String> {
    diagnostics(sources)
        .iter()
        .filter(|diagnostic| diagnostic.category == Category::Flaw)
        .map(|diagnostic| diagnostic.code.to_string())
        .collect()
}

fn diagnostics(sources: &[(&str, &str)]) -> Vec<Diagnostic> {
    package("exits", sources)
        .iter()
        .flat_map(|file| {
            file.all_flaws()
                .into_iter()
                .map(|(_, diagnostic)| diagnostic.clone())
                .chain(file.all_warnings().iter().cloned())
        })
        .collect()
}

#[test]
fn scalar_values_still_require_a_terminal_exit() {
    assert_eq!(
        check(&[("owner", "Number is in 0 to 10\n")]),
        ["type-terminal-exit-required"],
    );
}

#[test]
fn terminal_method_names_are_unrestricted() {
    for name in ["commit", "rollback", "finish"] {
        let flaws = check(&[(
            "owner",
            &format!("Resource has\n    value @()\n    {name}(self):\n        return\n"),
        )]);
        assert!(flaws.is_empty(), "{flaws:?}");
    }
}

#[test]
fn borrowing_or_transferring_methods_are_not_scalar_terminal_exits() {
    for mode in ["ref", "mut", "own"] {
        let flaws = check(&[(
            "owner",
            &format!(
                "Resource is in 0 to 10\nResource has\n    visit(self {mode}) Self: return self\n"
            ),
        )]);
        assert!(
            flaws
                .iter()
                .any(|code| code == "type-terminal-exit-required"),
            "{mode}: {flaws:?}"
        );
    }
}

#[test]
fn same_module_terminal_method_counts_across_files_in_either_order() {
    let sources = [
        ("owner", "Resource has\n    value @()\n"),
        ("owner", "Resource.finish(self):\n    return\n"),
    ];
    for sources in [sources, [sources[1], sources[0]]] {
        let flaws = check(&sources);
        assert!(flaws.is_empty(), "{flaws:?}");
    }
}

#[test]
fn foreign_terminal_function_cannot_supply_the_required_scalar_exit() {
    let flaws = check(&[
        ("owner", "Resource is in 0 to 10\n"),
        ("client", "forget(r Resource):\n    return\n"),
    ]);
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-terminal-exit-required"),
        "{flaws:?}"
    );
}

#[test]
fn aliases_use_the_original_types_exit() {
    let flaws = check(&[
        (
            "owner",
            "Resource has\n    value @()\n    commit(self):\n        return\n",
        ),
        ("client", "Alias is Resource\n"),
    ]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn generic_records_need_no_exit_covering_the_whole_type_family() {
    let flaws = check(&[(
        "owner",
        "Box(element) has\n    value element\n    finish(self):\n        return\n",
    )]);
    assert!(flaws.is_empty(), "{flaws:?}");
    let flaws = check(&[(
        "owner",
        "Item has\n    finish(self):\n        return\nBox(element) has value element\nBox(Item).finish(self):\n    return\n",
    )]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn empty_terminal_trait_defaults_are_rejected() {
    let flaws = check(&[(
        "owner",
        "Finish has\n    inspect(self ref) BigInt\n    finish(self):\n        return\nResource has Finish\n    value @()\n",
    )]);
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-empty-terminal-default"),
        "{flaws:?}"
    );
}

#[test]
fn abstract_terminal_contract_is_a_declared_exit() {
    let flaws = check(&[(
        "owner",
        "Finish has\n    finish(self)\nWrapped has Finish\n    value @()\n",
    )]);
    assert_eq!(flaws, ["type-missing-trait-method"]);
}

#[test]
fn a_refined_terminal_method_does_not_cover_every_value() {
    for body in ["", ":\n        return"] {
        let flaws = check(&[(
            "owner",
            &format!("Number is in 0 to 10\nNumber has\n    finish(self is in 1 to 10){body}\n"),
        )]);
        assert!(
            flaws
                .iter()
                .any(|code| code == "type-terminal-exit-required"),
            "{body}: {flaws:?}"
        );
    }
}

#[test]
fn a_terminal_parameter_named_self_must_accept_the_declared_type() {
    for body in ["", ":\n        return"] {
        let flaws = check(&[(
            "owner",
            &format!(
                "Other has\n    finish(self):\n        return\nResource is in 0 to 10\nResource has\n    finish(self Other){body}\n"
            ),
        )]);
        assert!(
            flaws
                .iter()
                .any(|code| code == "type-terminal-exit-required"),
            "{body}: {flaws:?}"
        );
    }
}

#[test]
fn same_module_free_function_does_not_supply_the_required_scalar_method() {
    let flaws = check(&[(
        "owner",
        "Resource is in 0 to 10\nfinish(r Resource):\n    return\n",
    )]);
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-terminal-exit-required"),
        "{flaws:?}"
    );
}

#[test]
fn const_generic_array_has_a_terminal_method_for_every_length() {
    let flaws = check(&[(
        "owner",
        "#fixed_array(element, count)\nBuffer(element, count BigInt) has\n    finish(self):\n        return\n",
    )]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn a_foreign_extension_method_does_not_grant_terminal_authority() {
    let flaws = check(&[
        ("owner", "Resource is in 0 to 10\n"),
        ("client", "Resource.finish(self):\n    return\n"),
    ]);
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-terminal-exit-required"),
        "{flaws:?}"
    );
}

#[test]
fn a_capability_types_own_exit_is_not_inherited_by_implementors() {
    let prefix =
        "Inspect has\n    inspect(self ref): return 0\nInspect.finish(self):\n    return\n";
    let flaws = check(&[(
        "owner",
        &format!("{prefix}Resource is in 0 to 10\nResource has Inspect\n"),
    )]);
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-terminal-exit-required"),
        "{flaws:?}"
    );
    let flaws = check(&[(
        "owner",
        &format!(
            "{prefix}Resource is in 0 to 10\nResource has Inspect\n    commit(self):\n        return\n"
        ),
    )]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn fieldless_interfaces_and_associated_functions_need_no_terminal_exit() {
    for source in [
        "Empty has\n",
        "Inspect has\n    inspect(self ref) BigInt\n",
        "Combine has\n    combine(left BigInt, right BigInt) BigInt\n",
        "Utility has\n    combine(left BigInt, right BigInt) BigInt: return 0\n",
        "Base has\n    inspect(self ref) BigInt\nDerived has Base\n",
    ] {
        let flaws = check(&[("owner", source)]);
        assert!(flaws.is_empty(), "{source}: {flaws:?}");
    }
}

#[test]
fn methods_only_extension_does_not_exempt_scalar_values() {
    let flaws = check(&[(
        "owner",
        "Resource is in 0 to 10\nResource has\n    inspect(self ref) BigInt: return 0\n",
    )]);
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-terminal-exit-required"),
        "{flaws:?}"
    );
}

#[test]
fn fieldless_terminal_requirements_and_borrowed_defaults_are_valid() {
    for method in [
        "finish(self)",
        "inspect(self ref):\n        return",
        "update(self mut):\n        return",
    ] {
        let flaws = check(&[("owner", &format!("Capability has\n    {method}\n"))]);
        assert!(flaws.is_empty(), "{method}: {flaws:?}");
    }
}

#[test]
fn empty_terminal_default_is_rejected_without_implementors() {
    let flaws = check(&[(
        "owner",
        "Finish has\n    inspect(self ref) BigInt\n    finish(self):\n        return\n",
    )]);
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-empty-terminal-default"),
        "{flaws:?}"
    );
}

#[test]
fn terminal_default_with_behavior_is_allowed() {
    let flaws = check(&[("owner", "Finish has\n    finish(self) BigInt: return 1\n")]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn concrete_record_extension_can_define_an_empty_terminal_method() {
    let flaws = check(&[(
        "owner",
        "Resource has value @()\nResource has\n    finish(self):\n        return\n",
    )]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn payload_free_sums_need_no_terminal_exit() {
    for source in [
        "State is Active or Done\n",
        "State is Active or Done\nAlias is State\n",
        "State(element) is Active or Done\n",
    ] {
        let flaws = check(&[("owner", source)]);
        assert!(flaws.is_empty(), "{source}: {flaws:?}");
    }
}

#[test]
fn payload_free_sum_fields_can_be_replaced() {
    let flaws = check(&[(
        "owner",
        "SessionState is Active or Closed\nSession has\n    state SessionState\n    enter(self mut):\n        self.state :: Active\n        return\n    close(self): return\n",
    )]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn replacing_a_sum_field_with_a_payload_requires_consumption() {
    let flaws = check(&[(
        "owner",
        "SessionState is Active(@SessionState) or Closed\nSession has\n    state SessionState\n    reset(self mut):\n        self.state :: Closed\n        return\n    close(self): return\n",
    )]);
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-rebind-would-discard-value"),
        "{flaws:?}"
    );
}

#[test]
fn empty_terminal_methods_on_payload_free_sums_get_help() {
    for source in [
        "Redraw is Changed or Unchanged\nRedraw has\n    release(self):\n        return\n",
        "Redraw is Changed or Unchanged\nRedraw.release(self):\n    return\n",
        "Redraw is Changed or Unchanged\nRedraw.finish(self): return\n",
        "Redraw(element) is Changed or Unchanged\nRedraw(element).release(self):\n    return\n",
        "Redraw is Changed or Unchanged\nAlias is Redraw\nAlias.release(self):\n    return\n",
    ] {
        let diagnostics = diagnostics(&[("owner", source)]);
        assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(
            diagnostic.code.slug().as_str(),
            "type-redundant-terminal-method"
        );
        assert_eq!(diagnostic.category, Category::Help);
        let name = &source[diagnostic.span.start() as usize..diagnostic.span.end() as usize];
        assert!(
            name.ends_with("release") || name.ends_with("finish"),
            "{name}"
        );
    }
}

#[test]
fn redundant_terminal_help_follows_methods_across_files() {
    let sources = [
        ("owner", "Redraw is Changed or Unchanged\n"),
        ("owner", "Redraw.release(self):\n    return\n"),
    ];
    for sources in [sources, [sources[1], sources[0]]] {
        let diagnostics = diagnostics(&sources);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(
            diagnostics[0].code.slug().as_str(),
            "type-redundant-terminal-method"
        );
        assert_eq!(diagnostics[0].category, Category::Help);
    }
}

#[test]
fn necessary_or_nonempty_methods_get_no_redundant_terminal_help() {
    for source in [
        "State is Active or Done\n",
        "Word is in 0 to 255\nWord.release(self):\n    return\n",
        "Word is in 0 to 255\nWord.release(self): return\nState is Active(Word) or Done\nState.release(self):\n    return\n",
        "Empty has\n    release(self):\n        return\n",
        "State is Active or Done\nState.inspect(self ref):\n    return\n",
        "State is Active or Done\nState.update(self mut):\n    return\n",
        "Word is in 0 to 255\nWord.release(self): return\nState is Active or Done\nState.release(self) Word: return 1\n",
        "State is Active or Done\nState.release(self):\n    work\n    return\nwork: return\n",
        "Finish has\n    release(self)\nState is Active or Done\nState has Finish\n    release(self):\n        return\n",
    ] {
        let diagnostics = diagnostics(&[("owner", source)]);
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
    }
}

#[test]
fn sums_with_payloads_need_no_terminal_exit() {
    for source in [
        "State is Active(@State) or Done\n",
        "State(element) is Active(element) or Done\n",
        "Resource has value @()\nResource.finish(self):\n    return\nState is Resource or Done\n",
    ] {
        let flaws = check(&[("owner", source)]);
        assert!(flaws.is_empty(), "{source}: {flaws:?}");
    }
}

#[test]
fn terminal_sum_method_can_ignore_a_payload_free_error() {
    let flaws = check(&[(
        "owner",
        "Byte is in 0 to 255\nByte.finish(self):\n    return\nError is Empty or Full\nAttempt is Value(Byte) or Failed(Error)\nAttempt has\n    finish(self):\n        when self is Value(value) then\n            return\n        return\n",
    )]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn fieldless_implementation_can_have_an_empty_terminal_method() {
    let flaws = check(&[(
        "owner",
        "Finish has\n    finish(self)\nConcrete has Finish\n    finish(self):\n        return\n",
    )]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn fields_with_abstract_methods_are_incomplete_concrete_types() {
    let flaws = check(&[(
        "owner",
        "Concrete has\n    value @()\n    inspect(self ref) BigInt\n    finish(self):\n        return\n",
    )]);
    assert_eq!(flaws, ["type-missing-trait-method"]);
}

#[test]
fn inherited_requirements_keep_partial_fieldless_implementations_abstract() {
    let flaws = check(&[(
        "owner",
        "Base has\n    inspect(self ref) BigInt\n    finish(self)\nPartial has Base\n    finish(self):\n        return\n",
    )]);
    assert_eq!(flaws, ["type-empty-terminal-default"]);
}

#[test]
fn implementing_the_last_requirement_allows_an_empty_terminal_method() {
    let flaws = check(&[(
        "owner",
        "Base has\n    inspect(self ref) BigInt\n    finish(self)\nComplete has Base\n    inspect(self ref) BigInt: return 1\n    finish(self):\n        return\n",
    )]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn separate_method_implementation_completes_a_fieldless_type() {
    let sources = [
        (
            "owner",
            "Complete has\n    inspect(self ref) BigInt\n    finish(self):\n        return\n",
        ),
        ("owner", "Complete.inspect(self ref) BigInt: return 1\n"),
    ];
    for sources in [sources, [sources[1], sources[0]]] {
        let flaws = check(&sources);
        assert!(flaws.is_empty(), "{flaws:?}");
    }
}
