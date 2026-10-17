mod support;

use diagnostic::Diagnostic;
use support::package;

const RESOURCES: &str = "Byte is in 0 to 255\nByte.finish(self): return\nTransaction has\n    state Byte\n    commit(self): return\n    rollback(self): return\n";
const WRAPPERS: &str = "use records.resources Transaction\nPair has\n    first Transaction\n    second Transaction\nBox(element) has value element\nNested has pair Pair\n";

fn check_modules(sources: &[(&str, &str)]) -> Vec<Diagnostic> {
    package("records", sources)
        .iter()
        .flat_map(|file| file.all_flaws())
        .map(|(_, flaw)| flaw.clone())
        .collect()
}

fn check(body: &str) -> Vec<Diagnostic> {
    let client = format!(
        "use records.resources Transaction\nuse records.wrappers Pair, Box, Nested\n{body}"
    );
    check_modules(&[
        ("resources", RESOURCES),
        ("wrappers", WRAPPERS),
        ("client", &client),
    ])
}

#[test]
fn field_terminal_methods_discharge_a_foreign_wrapper() {
    for body in [
        "process(pair Pair):\n    pair.first.commit\n    pair.second.rollback\n    return\n",
        "process(pair Pair):\n    pair.second.commit\n    pair.first.rollback\n    return\n",
        "process(pair Pair):\n    Pair(first, second) := pair\n    first.commit\n    second.rollback\n    return\n",
        "process(pair Pair) Transaction:\n    pair.first.commit\n    return pair.second\n",
        "forward(pair own Pair) Pair: return pair\n",
        "process(pair own Pair):\n    pair.first.commit\n    pair.second.rollback\n    return\n",
        "inspect(pair ref Pair): return\n",
    ] {
        let flaws = check(body);
        assert!(flaws.is_empty(), "{body}: {flaws:?}");
    }
}

#[test]
fn unaccessed_field_reports_its_path_type_operations_and_return_span() {
    let body = "process(pair Pair):\n    pair.first.commit\n    return\n";
    let client = format!(
        "use records.resources Transaction\nuse records.wrappers Pair, Box, Nested\n{body}"
    );
    let flaws = check(body);
    assert_eq!(flaws.len(), 1, "{flaws:?}");
    let flaw = &flaws[0];
    assert_eq!(
        flaw.code.slug().as_str(),
        "type-terminal-field-not-consumed"
    );
    assert_eq!(flaw.message, "`pair.second` is still pending");
    assert_eq!(flaw.arg("name"), Some("pair.second"));
    assert_eq!(flaw.arg("type_name"), Some("Transaction"));
    assert_eq!(
        flaw.help_on_span.as_deref(),
        Some("returning here leaves `pair.second` unconsumed")
    );
    assert_eq!(
        flaw.help.as_deref(),
        Some(
            "`pair.second` contains a `Transaction`; call its `commit` or `rollback` terminal method, or transfer ownership before returning"
        )
    );
    assert_eq!(flaw.span.start(), client.rfind("return").unwrap());
    assert_eq!(&client[flaw.span.start()..flaw.span.end()], "return");
}

#[test]
fn forgotten_wrappers_report_each_pending_field() {
    let flaws = check("process(pair Pair): return\n");
    let mut names: Vec<_> = flaws
        .iter()
        .map(|flaw| {
            assert_eq!(
                flaw.code.slug().as_str(),
                "type-terminal-field-not-consumed"
            );
            flaw.arg("name").unwrap()
        })
        .collect();
    names.sort();
    assert_eq!(names, ["pair.first", "pair.second"]);
}

#[test]
fn nested_and_generic_wrappers_preserve_field_obligations() {
    for body in [
        "process(value Nested):\n    value.pair.first.commit\n    value.pair.second.rollback\n    return\n",
        "process(value Box(Pair)):\n    value.value.first.commit\n    value.value.second.rollback\n    return\n",
        "process(value Box(Transaction)):\n    value.value.commit\n    return\n",
        "process(value Box(Box(Transaction))):\n    value.value.value.commit\n    return\n",
    ] {
        let flaws = check(body);
        assert!(flaws.is_empty(), "{body}: {flaws:?}");
    }
    for (body, field) in [
        (
            "process(value Nested):\n    value.pair.first.commit\n    return\n",
            "value.pair.second",
        ),
        (
            "process(value Box(Pair)):\n    value.value.first.commit\n    return\n",
            "value.value.second",
        ),
        ("process(value Box(Transaction)): return\n", "value.value"),
        (
            "process(value Box(Box(Transaction))): return\n",
            "value.value.value",
        ),
    ] {
        let flaws = check(body);
        assert_eq!(flaws.len(), 1, "{body}: {flaws:?}");
        assert_eq!(flaws[0].arg("name"), Some(field));
    }
}

#[test]
fn early_returns_and_branch_joins_preserve_remaining_fields() {
    let valid = "process(pair Pair, flag ref Byte):\n    when flag is 0 then\n        pair.first.rollback\n        pair.second.rollback\n        return\n    pair.first.commit\n    pair.second.commit\n    return\n";
    let invalid = "process(pair Pair, flag ref Byte):\n    pair.first.commit\n    when flag is 0 then\n        pair.second.rollback\n    return\n";
    assert!(check(valid).is_empty(), "{:?}", check(valid));
    let flaws = check(invalid);
    assert_eq!(flaws.len(), 1, "{flaws:?}");
    assert_eq!(flaws[0].arg("name"), Some("pair.second"));
}

#[test]
fn same_module_fields_still_require_explicit_handling() {
    for (body, pending) in [
        (
            "process(pair Pair):\n    pair.first.commit\n    pair.second.rollback\n    return\n",
            false,
        ),
        (
            "process(pair Pair):\n    pair.first.commit\n    return\n",
            true,
        ),
    ] {
        let source = format!(
            "{RESOURCES}{}{body}",
            WRAPPERS.replace("use records.resources Transaction\n", "")
        );
        let flaws = check_modules(&[("resources", &source)]);
        assert_eq!(!flaws.is_empty(), pending, "{flaws:?}");
    }
}

#[test]
fn records_without_resource_fields_need_no_terminal_method() {
    let flaws = check_modules(&[(
        "values",
        "Plain has value @()\nBox(element) has value element\nforget(value Plain): return\n",
    )]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn explicit_resource_lifecycles_keep_their_terminal_authority() {
    for body in [
        "forget(value Transaction): return\n",
        "forget(value Transaction):\n    Transaction(state) := value\n    return\n",
    ] {
        let flaws = check(body);
        assert!(!flaws.is_empty(), "{body}");
        assert!(
            flaws.iter().any(|flaw| matches!(
                flaw.code.slug().as_str(),
                "type-terminal-authority" | "type-terminal-value-not-consumed"
            )),
            "{flaws:?}"
        );
    }
}

#[test]
fn generic_calls_cannot_erase_record_field_obligations() {
    let flaws = check(
        "forget(element, value Box(element)): return\nprocess(tx Transaction):\n    wrapped Box(Transaction): Box(value: tx)\n    forget(Transaction, wrapped)\n    return\n",
    );
    assert!(
        flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "type-terminal-field-not-consumed"),
        "{flaws:?}"
    );
}

#[test]
fn moving_one_field_preserves_the_other_and_rejects_reuse() {
    for body in [
        "process(pair Pair):\n    tx: pair.first\n    tx.commit\n    return\n",
        "process(pair Pair):\n    pair.first.commit\n    pair.first.rollback\n    pair.second.commit\n    return\n",
        "process(pair ref Pair):\n    pair.first.commit\n    return\n",
    ] {
        let flaws = check(body);
        assert!(!flaws.is_empty(), "{body}");
        assert!(
            flaws.iter().any(|flaw| matches!(
                flaw.code.slug().as_str(),
                "type-terminal-field-not-consumed"
                    | "type-use-of-moved-value"
                    | "type-move-out-of-borrow"
            )),
            "{body}: {flaws:?}"
        );
    }
}

#[test]
fn private_wrapper_fields_remain_inaccessible() {
    let wrappers = WRAPPERS.replace(
        "    first Transaction",
        "    #private\n        first Transaction",
    );
    let flaws = check_modules(&[
        ("resources", RESOURCES),
        ("wrappers", &wrappers),
        (
            "client",
            "use records.wrappers Pair\nprocess(pair Pair):\n    Pair(first, second) := pair\n    first.commit\n    second.rollback\n    return\n",
        ),
    ]);
    assert!(
        flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "type-private-member"),
        "{flaws:?}"
    );
}

#[test]
fn early_return_diagnostic_points_to_the_incomplete_branch_exit() {
    let body = "process(pair Pair, flag ref Byte):\n    when flag is 0 then\n        pair.first.commit\n        return\n    pair.first.rollback\n    pair.second.rollback\n    return\n";
    let client = format!(
        "use records.resources Transaction\nuse records.wrappers Pair, Box, Nested\n{body}"
    );
    let flaws = check(body);
    assert_eq!(flaws.len(), 1, "{flaws:?}");
    assert_eq!(flaws[0].arg("name"), Some("pair.second"));
    assert_eq!(flaws[0].span.start(), client.find("return").unwrap());
    assert_eq!(
        &client[flaws[0].span.start()..flaws[0].span.end()],
        "return"
    );
}

#[test]
fn owned_wrapper_reports_the_pending_field_without_duplicate_wrapper_errors() {
    let flaws = check("process(pair own Pair):\n    pair.first.commit\n    return\n");
    assert_eq!(flaws.len(), 1, "{flaws:?}");
    assert_eq!(
        flaws[0].code.slug().as_str(),
        "type-terminal-field-not-consumed"
    );
    assert_eq!(flaws[0].arg("name"), Some("pair.second"));
}

#[test]
fn destructuring_cannot_discard_resource_fields() {
    for body in [
        "process(pair Pair):\n    Pair(second) := pair\n    second.rollback\n    return\n",
        "process(pair Pair):\n    when pair is Pair(_, second) then second.rollback\n    return\n",
        "process(pair Pair):\n    Pair(first, second) := pair\n    second.rollback\n    return\n",
    ] {
        let flaws = check(body);
        assert!(
            flaws.iter().any(|flaw| matches!(
                flaw.code.slug().as_str(),
                "type-pattern-would-discard-value"
                    | "type-terminal-value-not-consumed"
                    | "type-terminal-field-not-consumed"
            )),
            "{body}: {flaws:?}"
        );
    }
}

#[test]
fn fallthrough_diagnostic_points_to_the_end_of_the_function() {
    for body in [
        "process(pair Pair): pair.first.commit\n",
        "process(pair Pair):\n    pair.first.commit\n",
        "process(pair Pair):\n    inspect(pair)\n    pair.first.commit\ninspect(pair ref Pair): return\n",
    ] {
        let client = format!(
            "use records.resources Transaction\nuse records.wrappers Pair, Box, Nested\n{body}"
        );
        let flaws = check(body);
        assert_eq!(flaws.len(), 1, "{body}: {flaws:?}");
        let flaw = &flaws[0];
        assert_eq!(
            flaw.code.slug().as_str(),
            "type-terminal-field-not-consumed"
        );
        assert_eq!(flaw.message, "`pair.second` is still pending");
        assert_eq!(flaw.arg("name"), Some("pair.second"));
        assert_eq!(flaw.arg("type_name"), Some("Transaction"));
        assert_eq!(
            flaw.help_on_span.as_deref(),
            Some("function ends here with `pair.second` unconsumed")
        );
        assert_eq!(
            flaw.help.as_deref(),
            Some(
                "`pair.second` contains a `Transaction`; call its `commit` or `rollback` terminal method, or transfer ownership before the function ends"
            )
        );
        let end = client.find("pair.first.commit").unwrap() + "pair.first.commit".len();
        assert_eq!(flaw.span.start(), end);
        assert_eq!(flaw.span.end(), end);
    }
}

#[test]
fn fallthrough_after_handling_every_field_is_valid() {
    for body in [
        "process(pair Pair):\n    pair.first.commit\n    pair.second.rollback\n",
        "process(pair own Pair):\n    pair.first.commit\n    pair.second.rollback\n",
        "process(value Nested):\n    value.pair.first.commit\n    value.pair.second.rollback\n",
        "process(pair Pair, flag ref Byte):\n    pair.first.commit\n    when flag is 0 then pair.second.commit else pair.second.rollback\n",
        "process(pair Pair, flag ref Byte):\n    when flag is 0 then\n        pair.first.rollback\n        pair.second.rollback\n        return\n    pair.first.commit\n    pair.second.commit\n",
    ] {
        let flaws = check(body);
        assert!(flaws.is_empty(), "{body}: {flaws:?}");
    }
}

#[test]
fn conditional_fallthrough_preserves_pending_field_obligations() {
    for body in [
        "process(pair Pair, flag ref Byte):\n    pair.first.commit\n    when flag is 0 then pair.second.rollback\n",
        "process(pair Pair, flag ref Byte):\n    pair.first.commit\n    when flag is 0 then\n        pair.second.rollback\n        return\n",
    ] {
        let flaws = check(body);
        assert_eq!(flaws.len(), 1, "{body}: {flaws:?}");
        assert_eq!(flaws[0].arg("name"), Some("pair.second"));
        assert_eq!(
            flaws[0].help_on_span.as_deref(),
            Some("function ends here with `pair.second` unconsumed")
        );
    }
}

#[test]
fn terminal_methods_without_return_preserve_field_obligations() {
    for (operation, pending) in [
        ("self.first.commit", true),
        ("self.first.commit\n    self.second.rollback", false),
    ] {
        let wrappers = format!("{WRAPPERS}Pair.finish(self):\n    {operation}\n");
        let flaws = check_modules(&[("resources", RESOURCES), ("wrappers", &wrappers)]);
        if pending {
            assert_eq!(flaws.len(), 1, "{flaws:?}");
            assert_eq!(flaws[0].arg("name"), Some("self.second"));
            assert_eq!(
                flaws[0].help_on_span.as_deref(),
                Some("function ends here with `self.second` unconsumed")
            );
            assert_eq!(flaws[0].span.start(), wrappers.trim_end().len());
            assert!(flaws[0].span.is_empty());
        } else {
            assert!(flaws.is_empty(), "{flaws:?}");
        }
    }
}
