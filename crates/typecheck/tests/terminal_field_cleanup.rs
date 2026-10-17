mod support;

use support::check_with;
use typecheck::TypedExprKind;

const TRANSACTION: &str = r#"
Byte is in 0 to 255
Byte.finish(self):
    return
Transaction has
    state Byte
    commit(self):
        return
    rollback(self):
        return
"#;

fn expect_field_flaw(source: &str, field: &str) {
    let typed = check_with(TRANSACTION, source);
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| {
            flaw.code.slug() == "type-terminal-field-not-consumed"
                && flaw.arg("name") == Some(field)
        }),
        "{:?}",
        typed.all_flaws()
    );
}

fn expect_valid(source: &str) {
    let typed = check_with(TRANSACTION, source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn empty_terminal_method_cannot_discard_a_same_module_transaction() {
    expect_field_flaw(
        "Wrapper has\n    tx Transaction\n    release(self):\n        return\n",
        "self.tx",
    );
}

#[test]
fn field_terminal_methods_are_never_selected_implicitly() {
    for methods in [
        "finish(self):\n        return",
        "finish(self, state Byte):\n        return",
        "commit(self):\n        return\n    rollback(self):\n        return",
    ] {
        let typed = check_with(
            TRANSACTION,
            &format!(
                "Resource has\n    state Byte\n    {methods}\nWrapper has\n    resource Resource\n    release(self):\n        return\n"
            ),
        );
        assert!(
            support::flaw_codes(&typed)
                .iter()
                .any(|code| { code == "type-terminal-field-not-consumed" }),
            "{:?}",
            typed.all_flaws()
        );
        assert!(
            !typed
                .exprs
                .kind
                .iter()
                .any(|kind| matches!(kind, TypedExprKind::FnCall { args: Some(_), .. }))
        );
    }
}

#[test]
fn either_explicit_transaction_operation_handles_the_field() {
    for operation in ["commit", "rollback"] {
        expect_valid(&format!(
            "Wrapper has\n    tx Transaction\n    release(self):\n        self.tx.{operation}\n        return\n"
        ));
    }
}

#[test]
fn terminal_methods_can_accept_arguments_and_return_transferred_fields() {
    expect_valid(
        "Wrapper has\n    tx Transaction\n    finish(self, decision Byte) Transaction: return self.tx\n",
    );
}

#[test]
fn partial_field_consumption_leaves_the_other_field_pending() {
    expect_field_flaw(
        "Pair has\n    first Transaction\n    second Transaction\n    finish(self):\n        self.first.commit\n        return\n",
        "self.second",
    );
    expect_valid(
        "Pair has\n    first Transaction\n    second Transaction\n    finish(self):\n        self.second.rollback\n        self.first.commit\n        return\n",
    );
}

#[test]
fn one_call_cannot_consume_the_same_owned_field_twice() {
    let typed = check_with(
        TRANSACTION,
        "handle(first Transaction, second Transaction):\n    first.commit\n    second.rollback\n    return\nWrapper has\n    tx Transaction\n    finish(self):\n        handle(self.tx, self.tx)\n        return\n",
    );
    assert!(
        support::flaw_codes(&typed).contains(&"type-use-of-moved-value".to_owned()),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn early_returns_must_handle_every_owned_field() {
    expect_field_flaw(
        "Wrapper has\n    tx Transaction\n    finish(self, flag ref Byte):\n        when flag is 0 then\n            return\n        self.tx.commit\n        return\n",
        "self.tx",
    );
    expect_valid(
        "Wrapper has\n    tx Transaction\n    finish(self, flag ref Byte):\n        when flag is 0 then\n            self.tx.rollback\n            return\n        self.tx.commit\n        return\n",
    );
}

#[test]
fn conditional_consumption_does_not_discharge_the_other_path() {
    expect_field_flaw(
        "Wrapper has\n    tx Transaction\n    finish(self, flag ref Byte):\n        result: when flag is 0 then self.tx.commit else idle\n        return\nidle:\n    return\n",
        "self.tx",
    );
    expect_valid(
        "Wrapper has\n    tx Transaction\n    finish(self, flag ref Byte):\n        result: when flag is 0 then self.tx.commit else self.tx.rollback\n        return\n",
    );
}

#[test]
fn moving_a_field_to_a_local_preserves_its_obligation() {
    let typed = check_with(
        TRANSACTION,
        "Wrapper has\n    tx Transaction\n    finish(self):\n        moved: self.tx\n        return\n",
    );
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| {
            flaw.code.slug() == "type-terminal-value-not-consumed"
                && flaw.arg("name") == Some("moved")
        }),
        "{:?}",
        typed.all_flaws()
    );
    expect_valid(
        "Wrapper has\n    tx Transaction\n    finish(self):\n        moved: self.tx\n        moved.rollback\n        return\n",
    );
}

#[test]
fn destructuring_a_receiver_does_not_discard_its_fields() {
    let typed = check_with(
        TRANSACTION,
        "Wrapper has\n    tx Transaction\n    finish(self):\n        Wrapper(tx) := self\n        return\n",
    );
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| {
            flaw.code.slug() == "type-terminal-value-not-consumed" && flaw.arg("name") == Some("tx")
        }),
        "{:?}",
        typed.all_flaws()
    );
    expect_valid(
        "Wrapper has\n    tx Transaction\n    finish(self):\n        Wrapper(tx) := self\n        tx.rollback\n        return\n",
    );
}

#[test]
fn aliases_and_nested_containers_preserve_field_obligations() {
    expect_field_flaw(
        "Alias is Transaction\nWrapper has\n    tx Alias\n    finish(self):\n        return\n",
        "self.tx",
    );
    expect_field_flaw(
        "Inner has\n    tx Transaction\n    finish(self):\n        self.tx.commit\n        return\nOuter has\n    inner Inner\n    finish(self):\n        return\n",
        "self.inner",
    );
}

#[test]
fn arrays_require_handling_all_owned_elements() {
    expect_field_flaw(
        "Bundle has\n    items [Transaction; 2]\n    finish(self):\n        self.items.[0].commit\n        return\n",
        "self.items",
    );
    expect_valid(
        "Bundle has\n    items [Transaction; 2]\n    finish(self):\n        self.items.[1].rollback\n        self.items.[0].commit\n        return\n",
    );
    expect_valid("Bundle has\n    items [Transaction; 0]\n    finish(self):\n        return\n");
}

#[test]
fn owned_union_payloads_require_explicit_handling() {
    expect_field_flaw(
        "Maybe is Present(Transaction) or Absent\nMaybe has\n    finish(self):\n        return\n",
        "self",
    );
    expect_valid(
        "Maybe is Present(Transaction) or Absent\nMaybe has\n    finish(self): return when self is Present(tx) then tx.commit else idle\nidle:\n    return\n",
    );
    expect_field_flaw(
        "Maybe is Present(Transaction) or Failed(Transaction) or Absent\nMaybe has\n    finish(self): return when self is Present(tx) then tx.commit else idle\nidle:\n    return\n",
        "self",
    );
}

#[test]
fn union_fields_without_terminal_methods_require_explicit_payload_handling() {
    expect_field_flaw(
        "Maybe is Present(Transaction) or Absent\nWrapper has\n    value Maybe\n    finish(self): return\n",
        "self.value",
    );
    expect_valid(
        "Maybe is Present(Transaction) or Absent\nWrapper has\n    value Maybe\n    finish(self): return when self.value is Present(tx) then tx.commit else idle\nidle: return\n",
    );
    expect_valid(
        "Maybe is Present(Transaction) or Absent\nWrapper has\n    value Maybe\n    take(self) Maybe: return self.value\n",
    );
}

#[test]
fn union_fields_can_finish_payloads_and_fall_through_empty_variants() {
    for body in [
        "when self.value is Present(tx) then tx.commit\n        return",
        "when self.value is Present(tx) then\n            tx.commit\n            return\n        return",
        "return when self.value is Present(tx) then tx.commit else idle",
    ] {
        expect_valid(&format!(
            "Maybe is Present(Transaction) or Absent\nWrapper has\n    value Maybe\n    finish(self):\n        {body}\nidle: return\n"
        ));
    }
}

#[test]
fn naming_a_union_payload_preserves_its_terminal_obligation() {
    let typed = check_with(
        TRANSACTION,
        "Maybe is Present(Transaction) or Absent\nMaybe has\n    finish(self): return when self is Present(tx) then idle else idle\nidle:\n    return\n",
    );
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| {
            flaw.code.slug() == "type-terminal-value-not-consumed" && flaw.arg("name") == Some("tx")
        }),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn wildcard_payloads_cannot_discard_owned_resources() {
    expect_field_flaw(
        "Maybe is Present(first Transaction, second Transaction) or Absent\nMaybe has\n    finish(self): return when self is Present(tx, _) then tx.commit else idle\nidle:\n    return\n",
        "self",
    );
    expect_valid(
        "Maybe is Present(Transaction, Byte) or Absent\nMaybe has\n    finish(self): return when self is Present(tx, _) then tx.commit else idle\nidle:\n    return\n",
    );
}

#[test]
fn union_payloads_can_be_transferred_or_borrowed() {
    expect_valid(
        "Maybe is Present(Transaction) or Absent\nMaybe has\n    take(self) Transaction: return when self is Present(tx) then tx else Transaction(state: 0)\n",
    );
    expect_valid(
        "Maybe is Present(Transaction) or Absent\nMaybe has\n    finish(self): return when self is Present(tx) then tx.rollback else idle\ninspect(value ref Transaction):\n    return\nobserve(value ref Maybe): return when value is Present(tx) then inspect(tx) else idle\nidle:\n    return\n",
    );
}

#[test]
fn release_contracts_do_not_authorize_implicit_field_calls() {
    expect_field_flaw(
        "Release has\n    release(self)\nResource has Release\n    state Byte\n    release(self):\n        return\nWrapper has\n    resource Resource\n    release(self):\n        return\n",
        "self.resource",
    );
}

#[test]
fn borrowed_fields_and_primitive_storage_do_not_require_terminal_calls() {
    expect_valid(
        "Wrapper has\n    tx ref Transaction\n    pointer @Byte\n    state Byte\n    finish(self):\n        return\n",
    );
}

#[test]
fn ordinary_defining_module_terminal_functions_keep_their_authority() {
    expect_valid("commit(tx Transaction):\n    return\n");
}

#[test]
fn specialized_terminal_methods_preserve_owned_field_obligations() {
    expect_field_flaw(
        "Container(element) has\n    value element\n    finish(self):\n        return\nconsume(value Container(Transaction)):\n    value.finish\n    return\n",
        "self.value",
    );
    expect_valid(
        "Container(element) has\n    value element\n    finish(self):\n        self.value.rollback\n        return\nconsume(value Container(Transaction)):\n    value.finish\n    return\n",
    );
}

#[test]
fn conditional_destructuring_preserves_resource_obligations() {
    let typed = check_with(
        TRANSACTION,
        "Maybe is Present(Transaction) or Absent\nMaybe has\n    finish(self) Maybe:\n        when self is Present(tx) then\n            return Absent\n        return self\n",
    );
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| {
            flaw.code.slug() == "type-terminal-value-not-consumed" && flaw.arg("name") == Some("tx")
        }),
        "{:?}",
        typed.all_flaws()
    );
    expect_valid(
        "Maybe is Present(Transaction) or Absent\nMaybe has\n    finish(self) Maybe:\n        when self is Present(tx) then\n            tx.commit\n            return Absent\n        return self\n",
    );
}

#[test]
fn conditional_wildcards_cannot_discard_resources() {
    let typed = check_with(
        TRANSACTION,
        "Maybe is Present(first Transaction, second Transaction) or Absent\nMaybe has\n    finish(self) Maybe:\n        when self is Present(tx, _) then\n            tx.commit\n            return Absent\n        return self\n",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-terminal-field-not-consumed"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn conditional_borrowed_patterns_preserve_their_owner() {
    expect_valid(
        "Maybe is Present(Transaction) or Absent\nMaybe has\n    finish(self): return when self is Present(tx) then tx.commit else idle\nobserve(value ref Maybe):\n    when value is Present(tx) then\n        inspect(tx)\n        return\n    return\ninspect(value ref Transaction):\n    value.state\n    return\nrun:\n    value Maybe: Present(Transaction(state: 1))\n    observe(value)\n    value.finish\n    return\nidle:\n    return\n",
    );
}

#[test]
fn borrowed_pattern_payloads_cannot_be_consumed() {
    for permission in ["ref", "mut"] {
        for body in [
            "when value is Present(tx) then\n        tx.commit\n        return\n    return",
            "when value is Present(tx) then tx.commit else idle\n    return",
            "when value is Present(tx) and flag is 0 then tx.commit else idle\n    return",
            "while value is Present(tx) do\n        tx.commit\n    return",
        ] {
            let typed = check_with(
                TRANSACTION,
                &format!(
                    "Maybe is Present(Transaction) or Absent\nMaybe.finish(self): return when self is Present(tx) then tx.commit else idle\nobserve(value {permission} Maybe, flag ref Byte):\n    {body}\nidle:\n    return\n"
                ),
            );
            assert!(
                support::flaw_codes(&typed)
                    .iter()
                    .any(|code| code == "type-move-out-of-borrow"),
                "{permission}: {body}: {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn borrowed_resources_cannot_supply_consuming_calls() {
    for permission in ["ref", "mut"] {
        for body in ["value.commit", "take(value)", "take(deref value)"] {
            let typed = check_with(
                TRANSACTION,
                &format!(
                    "take(value own Transaction) Transaction: return value\nobserve(value {permission} Transaction): {return_prefix}{body}\n",
                    return_prefix = if body.starts_with('\n') {
                        ""
                    } else {
                        "return "
                    }
                ),
            );
            assert!(
                support::flaw_codes(&typed)
                    .iter()
                    .any(|code| code == "type-move-out-of-borrow"),
                "{permission}: {body}: {:?}",
                typed.all_flaws()
            );
        }
        let typed = check_with(
            TRANSACTION,
            &format!(
                "Wrapper has\n    tx Transaction\n    finish(self): return self.tx.commit\nobserve(value {permission} Wrapper): return value.tx.commit\n"
            ),
        );
        assert!(
            support::flaw_codes(&typed)
                .iter()
                .any(|code| code == "type-move-out-of-borrow"),
            "{permission}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn borrowed_resources_cannot_be_copied_into_owned_locals_or_results() {
    for permission in ["ref", "mut"] {
        for body in [
            "copy(value PERMISSION Transaction) Transaction: return deref value\n",
            "copy(value PERMISSION Transaction):\n    duplicate: deref value\n    duplicate.commit\n    return\n",
        ] {
            let typed = check_with(TRANSACTION, &body.replace("PERMISSION", permission));
            assert!(
                support::flaw_codes(&typed)
                    .iter()
                    .any(|code| code == "type-move-out-of-borrow"),
                "{permission}: {body}: {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn references_can_be_forwarded_without_consuming_the_resource() {
    expect_valid(
        "pass(value ref Transaction) ref Transaction: return value\ninspect(value ref Transaction):\n    value.state\n    return\nrun:\n    value: Transaction(state: 1)\n    shared: pass(value)\n    inspect(shared)\n    value.commit\n    return\n",
    );
}

#[test]
fn mutable_resource_replacement_consumes_the_previous_value_explicitly() {
    expect_valid(
        "Wrapper has\n    tx Transaction\n    finish(self): return self.tx.commit\nreplace(previous Transaction, next Transaction) Transaction:\n    previous.rollback\n    return next\nupdate(value mut Wrapper, next Transaction):\n    value.tx:: replace(value.tx, next)\n    return\nrun:\n    value: Wrapper(tx: Transaction(state: 1))\n    update(value, Transaction(state: 2))\n    value.finish\n    return\n",
    );
}

#[test]
fn borrowed_list_elements_cannot_be_consumed() {
    for permission in ["ref", "mut"] {
        for body in ["head.commit", "tail.[0].rollback"] {
            let typed = check_with(
                TRANSACTION,
                &format!(
                    "observe(value {permission} [Transaction; 2]): return when value is [head, ...tail] then {body}\n"
                ),
            );
            assert!(
                support::flaw_codes(&typed)
                    .iter()
                    .any(|code| code == "type-move-out-of-borrow"),
                "{permission}: {body}: {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn conditional_when_arms_preserve_resource_obligations() {
    let typed = check_with(
        TRANSACTION,
        "Maybe is Present(Transaction) or Absent\nMaybe has\n    finish(self, flag ref Byte): return when self is Present(tx) and flag is 0 then idle else idle\nidle:\n    return\n",
    );
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| {
            flaw.code.slug() == "type-terminal-value-not-consumed" && flaw.arg("name") == Some("tx")
        }),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn list_patterns_must_handle_both_owned_head_and_tail() {
    for body in ["idle", "head.commit", "tail.[0].rollback"] {
        let typed = check_with(
            TRANSACTION,
            &format!(
                "Bundle has\n    items [Transaction; 2]\n    finish(self): return when self.items is [head, ...tail] then {body}\nidle:\n    return\n"
            ),
        );
        assert!(
            support::flaw_codes(&typed)
                .iter()
                .any(|code| code.starts_with("type-terminal-")),
            "{body}: {:?}",
            typed.all_flaws()
        );
    }
    expect_valid(
        "Bundle has\n    items [Transaction; 2]\n    finish(self): return when self.items is [head, ...tail] then finish_parts(head, tail)\nfinish_parts(head Transaction, tail [Transaction; 1]):\n    head.commit\n    tail.[0].rollback\n    return\nidle:\n    return\n",
    );
}

#[test]
fn list_wildcards_cannot_discard_owned_elements() {
    for pattern in ["[head, ..._]", "[_, ...tail]"] {
        expect_field_flaw(
            &format!(
                "Bundle has\n    items [Transaction; 2]\n    finish(self): return when self.items is {pattern} then idle\nidle:\n    return\n"
            ),
            "self.items",
        );
    }
}

#[test]
fn repeating_loops_cannot_release_an_owned_field_twice() {
    let typed = check_with(
        TRANSACTION,
        "Wrapper has\n    tx Transaction\n    finish(self, flag ref Byte):\n        while flag is 1 do\n            self.tx.commit\n        return\n",
    );
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code.contains("moved-value")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn loop_pattern_bindings_preserve_resource_obligations() {
    let typed = check_with(
        TRANSACTION,
        "Maybe is Present(Transaction) or Absent\nMaybe has\n    finish(self) Maybe:\n        while self is Present(tx) do\n            idle\n        return self\nidle:\n    return\n",
    );
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| {
            flaw.code.slug() == "type-terminal-value-not-consumed" && flaw.arg("name") == Some("tx")
        }),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn loop_early_returns_must_handle_remaining_fields() {
    expect_field_flaw(
        "Pair has\n    first Transaction\n    second Transaction\n    finish(self, flag ref Byte, choice ref Byte):\n        while flag is 1 do\n            when choice is 0 then\n                self.first.commit\n                return\n        self.first.rollback\n        self.second.rollback\n        return\n",
        "self.second",
    );
    expect_valid(
        "Pair has\n    first Transaction\n    second Transaction\n    finish(self, flag ref Byte, choice ref Byte):\n        while flag is 1 do\n            when choice is 0 then\n                self.first.commit\n                self.second.commit\n                return\n        self.first.rollback\n        self.second.rollback\n        return\n",
    );
}

#[test]
fn iteration_locals_require_explicit_cleanup() {
    let typed = check_with(
        TRANSACTION,
        "Wrapper has\n    tx Transaction\n    finish(self, flag ref Byte):\n        while flag is 1 do\n            local: Transaction(state: 0)\n        self.tx.commit\n        return\n",
    );
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| flaw.code.slug()
            == "type-terminal-value-not-consumed"
            && flaw.arg("name") == Some("local")),
        "{:?}",
        typed.all_flaws()
    );
    expect_valid(
        "Wrapper has\n    tx Transaction\n    finish(self, flag ref Byte):\n        while flag is 1 do\n            local: Transaction(state: 0)\n            local.rollback\n        self.tx.commit\n        return\n",
    );
}

#[test]
fn terminal_calls_respect_borrow_dependencies_in_owned_fields() {
    let header = "Dependent has\n    resource ref Transaction\n    finish(self):\n        inspect(self.resource)\n        return\ninspect(value ref Transaction):\n    value.state\n    return\nWrapper has\n    dependent Dependent\n    finish(self):\n        self.dependent.finish\n        return\nrun:\n    owner: Transaction(state: 0)\n    wrapper: Wrapper(dependent: Dependent(resource: owner))\n    wrapper.finish\n    owner.commit\n    return\n";
    expect_valid(header);
    let typed = check_with(
        TRANSACTION,
        &header.replace(
            "wrapper.finish\n    owner.commit",
            "owner.commit\n    wrapper.finish",
        ),
    );
    assert!(
        support::flaw_codes(&typed).contains(&"type-use-of-invalidated-reference".to_owned()),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn pointer_replacement_restores_consumed_element_storage() {
    expect_valid(
        "replace(previous Byte, next Byte) Byte: return next\nwrite(pointer mut @Byte, index ref Byte):\n    pointer.[index]:: replace(pointer.[index], 1)\n    pointer.[index]:: replace(pointer.[index], 2)\n    return\n",
    );
}

#[test]
fn conditional_locals_still_require_explicit_terminal_cleanup() {
    for (cleanup, valid) in [("value.commit", true), ("", false)] {
        let typed = check_with(
            TRANSACTION,
            &format!(
                "Wrapper has\n    tx Transaction\n    finish(self, condition ref Byte):\n        when condition = 1 then\n            value: self.tx\n            {cleanup}\n        else self.tx.rollback\n        return\n"
            ),
        );
        let missing = typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-terminal-value-not-consumed");
        assert_eq!(missing, !valid, "{:?}", typed.all_flaws());
    }
}
