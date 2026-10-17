mod support;

use support::package_flaws;

const DATABASE: &str = "Word is in 0 to 255\nWord has\n    finish(self):\n        return\nTransaction has\n    state Word\n    finish(self):\n        return\ncommit(tx Transaction):\n    return\nrollback(tx Transaction):\n    return\n";
const CLIENT: &str = "client";
const DATABASE_MODULE: &str = "database";
const TRANSACTION_USE: &str = "use transactions.database Transaction\n";
const MAYBE: &str =
    "use transactions.database Transaction\nMaybe is Present(Transaction) or Absent\n";
const CHOICE: &str = "use transactions.database Transaction\nChoice is First(Transaction) or Second(Transaction) or Empty or Done\n";
const NESTED: &str = "use transactions.database Transaction\nMaybe(element) is Present(element) or Absent\nNested is Wrapped(Maybe(Transaction)) or Empty\n";
const NOT_CONSUMED: &str = "type-terminal-value-not-consumed";
const FIELD_NOT_CONSUMED: &str = "type-terminal-field-not-consumed";
const TERMINAL_AUTHORITY: &str = "type-terminal-authority";

fn check(module: &str, source: &str) -> Vec<String> {
    package_flaws("transactions", &[("database", DATABASE), (module, source)])
}

enum Expectation {
    Empty,
    All(&'static str),
    Any(&'static str),
    AnyOf(&'static [&'static str]),
}

const CASES: &[(&str, &str, &str, &[&str], Expectation)] = &[
    (
        "defining_module_can_provide_terminal_exits_in_another_file",
        DATABASE_MODULE,
        "",
        &["close(tx Transaction):\n    return\n"],
        Expectation::Empty,
    ),
    (
        "external_terminal_parameters_must_delegate_to_an_exit",
        CLIENT,
        "",
        &[
            "finish(tx Transaction):\n    commit(tx)\n    return\n",
            "finish(tx Transaction):\n    rollback(tx)\n    return\n",
        ],
        Expectation::Empty,
    ),
    (
        "external_terminal_parameters_must_delegate_to_an_exit",
        CLIENT,
        "",
        &["forget(tx Transaction):\n    return\n"],
        Expectation::Any(NOT_CONSUMED),
    ),
    (
        "aliases_and_wrappers_do_not_grant_terminal_authority",
        CLIENT,
        "",
        &[
            "Alias is Transaction\nforget(tx Alias):\n    return\n",
            "Wrapper has tx Transaction\nforget(value Wrapper):\n    return\n",
            "Wrapper has tx Transaction\nforget(tx Transaction):\n    wrapped: Wrapper(tx)\n    return\n",
            "Wrapper has tx Transaction\nforget(value Wrapper):\n    Wrapper(tx) := value\n    return\n",
        ],
        Expectation::AnyOf(&[NOT_CONSUMED, FIELD_NOT_CONSUMED]),
    ),
    (
        "moving_through_a_local_preserves_the_terminal_obligation",
        CLIENT,
        "",
        &["finish(tx Transaction):\n    moved: tx\n    commit(moved)\n    return\n"],
        Expectation::Empty,
    ),
    (
        "moving_through_a_local_preserves_the_terminal_obligation",
        CLIENT,
        "",
        &["forget(tx Transaction):\n    moved: tx\n    return\n"],
        Expectation::Any(NOT_CONSUMED),
    ),
    (
        "foreign_results_cannot_be_ignored",
        CLIENT,
        "",
        &[
            "forward(tx own Transaction) Transaction: return tx\nforget(tx Transaction):\n    forward(tx)\n    return\n",
        ],
        Expectation::Any(NOT_CONSUMED),
    ),
    (
        "returning_ownership_does_not_require_terminal_authority",
        CLIENT,
        "",
        &["forward(tx own Transaction) Transaction: return tx\n"],
        Expectation::Empty,
    ),
    (
        "unions_without_terminal_methods_preserve_payload_obligations",
        CLIENT,
        MAYBE,
        &[
            "forget(value Maybe): return\n",
            "forget(tx Transaction):\n    value Maybe: Present(tx)\n    return\n",
            "forget(value Maybe):\n    when value is Present(tx) then\n        return\n    return\n",
            "forget(value Maybe):\n    when value is Present(_) then\n        return\n    return\n",
            "forward(value own Maybe) Maybe: return value\nforget(value Maybe):\n    forward(value)\n    return\n",
        ],
        Expectation::All(NOT_CONSUMED),
    ),
    (
        "unions_without_terminal_methods_can_finish_transfer_or_borrow_payloads",
        CLIENT,
        MAYBE,
        &[
            "finish(value Maybe): return when value is\n    Present(tx) then commit(tx)\n    Absent then idle\nidle: return\n",
            "finish(value Maybe): return when value is\n    Present(tx) then rollback(tx)\n    Absent then idle\nidle: return\n",
            "forward(value own Maybe) Maybe: return value\n",
            "take(value own Maybe) Transaction: return when value is\n    Present(tx) then tx\n    Absent then Transaction(state: 0)\n",
            "inspect(value ref Maybe):\n    when value is Present(tx) then\n        inspect_tx(tx)\n    return\ninspect_tx(tx ref Transaction): return\n",
        ],
        Expectation::Empty,
    ),
    (
        "handling_union_payloads_leaves_no_empty_variant_obligation",
        CLIENT,
        MAYBE,
        &[
            "finish(value Maybe):\n    when value is Present(tx) then commit(tx)\n    return\n",
            "finish(value Maybe):\n    when value is Present(tx) then\n        commit(tx)\n        return\n    return\n",
            "finish(value Maybe): return when value is Present(tx) then commit(tx) else idle\nidle: return\n",
            "finish(value Maybe):\n    when value is\n        Present(tx) then commit(tx)\n        else idle\n    return\nidle: return\n",
            "finish(value Maybe):\n    when value is\n        Present(tx) then commit(tx)\n    return\n",
            "finish(value Maybe):\n    when value is Absent then return\n    when value is Present(tx) then commit(tx)\n    return\n",
            "finish(value Maybe) Maybe:\n    when value is Present(tx) then\n        commit(tx)\n        return Absent\n    return value\n",
        ],
        Expectation::Empty,
    ),
    (
        "handling_multiple_union_payloads_leaves_only_empty_variants",
        CLIENT,
        CHOICE,
        &[
            "finish(value Choice):\n    when value is First(tx) then\n        commit(tx)\n        return\n    when value is Second(tx) then\n        rollback(tx)\n        return\n    return\n",
            "finish(value Choice):\n    when value is\n        First(tx) then commit(tx)\n        Second(tx) then rollback(tx)\n    return\n",
            "finish(value Choice): return when value is\n    First(tx) then commit(tx)\n    Second(tx) then rollback(tx)\n    else idle\nidle: return\n",
        ],
        Expectation::Empty,
    ),
    (
        "conditional_payload_handling_cannot_discharge_other_paths",
        CLIENT,
        MAYBE,
        &[
            "finish(value Maybe, flag ref Word): return when value is Present(tx) and flag is 0 then commit(tx) else idle\nidle: return\n",
            "finish(value Maybe, flag ref Word):\n    when flag is 0 then\n        when value is Present(tx) then commit(tx)\n    return\n",
            "finish(value Maybe):\n    when value is Present(_) then idle\n    return\nidle: return\n",
        ],
        Expectation::All(NOT_CONSUMED),
    ),
    (
        "constrained_payload_matches_leave_the_other_payload_values_pending",
        CLIENT,
        "",
        &[
            "use transactions.database Transaction\nCode is in 0 to 1\nCode.finish(self): return\nMaybe is Present(Transaction, Code) or Absent\nfinish(value Maybe): return when value is Present(tx, 0) then commit(tx) else idle\nidle: return\n",
        ],
        Expectation::All(NOT_CONSUMED),
    ),
    (
        "replacing_an_empty_union_restores_its_payload_obligation",
        CLIENT,
        MAYBE,
        &[
            "replace(previous Maybe, next own Maybe) Maybe:\n    when previous is Present(old) then commit(old)\n    return next\nHolder has value Maybe\nHolder.finish(self):\n    when self.value is Present(tx) then commit(tx)\n    return\nrefill(holder mut Holder, next own Maybe):\n    holder.value :: replace(holder.value, next)\n    return\ngate(holder mut Holder, next own Maybe) True or False:\n    refill(holder, next)\n    return True\nrun(initial own Maybe, tx Transaction):\n    holder: Holder(value: initial)\n    when holder.value is Present(old) then\n        commit(old)\n        rollback(tx)\n        return\n    next Maybe: Present(tx)\n    refill(holder, next)\n    holder.finish\n    return\n",
        ],
        Expectation::Empty,
    ),
    (
        "replacing_an_empty_union_restores_its_payload_obligation",
        CLIENT,
        MAYBE,
        &[
            "replace(previous Maybe, next own Maybe) Maybe:\n    when previous is Present(old) then commit(old)\n    return next\nHolder has value Maybe\nHolder.finish(self):\n    when self.value is Present(tx) then commit(tx)\n    return\nrefill(holder mut Holder, next own Maybe):\n    holder.value :: replace(holder.value, next)\n    return\ngate(holder mut Holder, next own Maybe) True or False:\n    refill(holder, next)\n    return True\nrun(initial own Maybe, tx Transaction):\n    holder: Holder(value: initial)\n    when holder.value is Present(old) then\n        commit(old)\n        rollback(tx)\n        return\n    next Maybe: Present(tx)\n    refill(holder, next)\n    \n    return\n",
            "replace(previous Maybe, next own Maybe) Maybe:\n    when previous is Present(old) then commit(old)\n    return next\nHolder has value Maybe\nHolder.finish(self):\n    when self.value is Present(tx) then commit(tx)\n    return\nrefill(holder mut Holder, next own Maybe):\n    holder.value :: replace(holder.value, next)\n    return\ngate(holder mut Holder, next own Maybe) True or False:\n    refill(holder, next)\n    return True\nrun(initial own Maybe, tx Transaction):\n    holder: Holder(value: initial)\n    when holder.value is Present(old) then\n        commit(old)\n        rollback(tx)\n        return\n    next Maybe: Present(tx)\n    when holder.value is Absent and gate(holder, next) is True then return\n    holder.finish\n    return\n",
        ],
        Expectation::All(NOT_CONSUMED),
    ),
    (
        "unions_without_terminal_methods_must_handle_every_payload_and_path",
        CLIENT,
        TRANSACTION_USE,
        &[
            "Choice is First(Transaction) or Second(Transaction) or Empty\nfinish(value Choice):\n    when value is First(tx) then\n        commit(tx)\n    return\n",
            "Pair is Both(Transaction, Transaction) or Empty\nfinish(value Pair):\n    when value is Both(tx, _) then\n        commit(tx)\n    return\n",
            "Maybe is Present(Transaction) or Absent\nfinish(value Maybe, flag ref Word):\n    when value is Present(tx) then\n        when flag is 0 then\n            commit(tx)\n        return\n    return\n",
        ],
        Expectation::All(NOT_CONSUMED),
    ),
    (
        "generic_and_nested_unions_preserve_payload_obligations",
        CLIENT,
        NESTED,
        &[
            "finish(value Nested): return when value is\n    Wrapped(inner) then finish_inner(inner)\n    Empty then idle\nfinish_inner(value Maybe(Transaction)): return when value is\n    Present(tx) then commit(tx)\n    Absent then idle\nidle: return\n",
            "finish(value Nested):\n    when value is Wrapped(inner) then\n        when inner is Present(tx) then commit(tx)\n    return\n",
        ],
        Expectation::Empty,
    ),
    (
        "generic_and_nested_unions_preserve_payload_obligations",
        CLIENT,
        NESTED,
        &[
            "forget(value Nested): return\n",
            "forget(element, value Maybe(element)): return\nrun(value Maybe(Transaction)): return forget(Transaction, value)\n",
            "forget(value Nested):\n    when value is Wrapped(inner) then\n        when inner is Present(tx) then\n            return\n    return\n",
        ],
        Expectation::All(NOT_CONSUMED),
    ),
    (
        "borrowing_does_not_create_a_terminal_obligation",
        CLIENT,
        "",
        &[
            "inspect(tx ref Transaction) Word: return tx.state\n",
            "inspect(tx ref Transaction) Word:\n    Transaction(state) := tx\n    return state\n",
            "Wrapper has\n    tx Transaction\n    finish(self):\n        commit(self.tx)\n        return\ninspect(value ref Wrapper) Word:\n    Wrapper(tx) := value\n    return tx.state\n",
        ],
        Expectation::Empty,
    ),
    (
        "loop_local_values_must_finish_each_iteration",
        CLIENT,
        "",
        &[
            "run:\n    for index in 0 to 3\n        tx: Transaction(state: 0)\n        commit(tx)\n    return\n",
        ],
        Expectation::Empty,
    ),
    (
        "loop_local_values_must_finish_each_iteration",
        CLIENT,
        "",
        &[
            "run:\n    for index in 0 to 3\n        tx: Transaction(state: 0)\n        \n    return\n",
        ],
        Expectation::Any(NOT_CONSUMED),
    ),
    (
        "arrays_preserve_each_elements_terminal_obligation",
        CLIENT,
        "",
        &[
            "finish(values [Transaction; 2]):\n    commit(values.[0])\n    rollback(values.[1])\n    return\n",
        ],
        Expectation::Empty,
    ),
    (
        "arrays_preserve_each_elements_terminal_obligation",
        CLIENT,
        "",
        &["finish(values [Transaction; 2]):\n    commit(values.[0])\n    return\n"],
        Expectation::Any(NOT_CONSUMED),
    ),
    (
        "external_code_cannot_destructure_a_resource_to_discard_it",
        CLIENT,
        "",
        &[
            "forget(tx Transaction):\n    Transaction(state) := tx\n    return\n",
            "forget(tx Transaction) Word: return when tx is Transaction(state) then state\n",
        ],
        Expectation::Any(TERMINAL_AUTHORITY),
    ),
    (
        "wrapper_can_explicitly_finish_its_foreign_field",
        CLIENT,
        "",
        &[
            "Wrapper has tx Transaction\nWrapper.finish(self):\n    commit(self.tx)\n    return\n",
            "Wrapper has tx Transaction\nWrapper.finish(self):\n    Wrapper(tx) := self\n    commit(tx)\n    return\n",
        ],
        Expectation::Empty,
    ),
    (
        "early_return_can_complete_or_return_the_obligation",
        CLIENT,
        "",
        &[
            "finish(tx Transaction, flag ref Word):\n    when flag is 0 then\n        rollback(tx)\n        return\n    commit(tx)\n    return\n",
            "forward(tx own Transaction, flag ref Word) Transaction:\n    when flag is 0 then\n        return tx\n    return tx\n",
        ],
        Expectation::Empty,
    ),
    (
        "terminal_delegation_is_checked_on_early_returns",
        CLIENT,
        "",
        &[
            "finish(tx Transaction, flag ref Word):\n    when flag is 0 then\n        return\n    commit(tx)\n    return\n",
        ],
        Expectation::Any(NOT_CONSUMED),
    ),
    (
        "generic_specialization_keeps_the_defining_module_authority",
        CLIENT,
        "",
        &[
            "forget(element, value element):\n    return\nfinish(tx Transaction):\n    forget(Transaction, tx)\n    return\n",
        ],
        Expectation::Any(NOT_CONSUMED),
    ),
];

#[test]
fn terminal_authority_cases() {
    for &(name, module, prelude, bodies, ref expectation) in CASES {
        for body in bodies {
            let source = format!("{prelude}{body}");
            let flaws = check(module, &source);
            match expectation {
                Expectation::Empty => {
                    assert!(
                        flaws.is_empty(),
                        "case `{name}` failed:\n{source}\n{flaws:?}"
                    );
                }
                Expectation::All(code) => {
                    assert!(!flaws.is_empty(), "case `{name}` failed:\n{source}");
                    assert!(
                        flaws.iter().all(|actual| actual == code),
                        "case `{name}` failed:\n{source}\n{flaws:?}"
                    );
                }
                Expectation::Any(code) => assert!(
                    flaws.iter().any(|actual| actual == code),
                    "case `{name}` failed:\n{source}\n{flaws:?}"
                ),
                Expectation::AnyOf(codes) => assert!(
                    flaws.iter().any(|actual| codes.contains(&actual.as_str())),
                    "case `{name}` failed:\n{source}\n{flaws:?}"
                ),
            }
        }
    }
}
