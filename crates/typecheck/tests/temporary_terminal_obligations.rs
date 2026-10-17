mod support;

use diagnostic::Diagnostic;

const RESOURCES: &str = r#"Byte is in 0 to 255
Byte.finish(self): return
Transaction has
    state Byte
    commit(self): return
"#;
const CONTAINERS: &str = r#"use temporaries.resources Transaction, Byte
Pair has
    first Transaction
    second Transaction
Single has value Transaction
Nested has value Pair
Maybe is Present(Transaction) or Absent
Mixed has
    value Transaction
    state Byte
make(first own Transaction, second own Transaction) Pair: return Pair(first: first, second: second)
finish(pair Pair):
    pair.first.commit
    pair.second.commit
    return
"#;

fn check(body: &str) -> Vec<Diagnostic> {
    let client = format!("{CONTAINERS}{body}");
    support::package(
        "temporaries",
        &[("resources", RESOURCES), ("client", &client)],
    )
    .iter()
    .flat_map(|file| file.all_flaws())
    .map(|(_, flaw)| flaw.clone())
    .collect()
}

fn expect_pending(body: &str) {
    let flaws = check(body);
    assert!(
        flaws.iter().any(|flaw| matches!(
            flaw.code.slug().as_str(),
            "type-terminal-field-not-consumed" | "type-terminal-value-not-consumed"
        )),
        "{body}: {flaws:?}"
    );
}

#[test]
fn temporary_record_projections_preserve_unselected_fields() {
    for body in [
        r#"run(first Transaction, second Transaction):
    Pair(first: first, second: second).first.commit
    return
"#,
        r#"run(first Transaction, second Transaction) Transaction:
    return Pair(first: first, second: second).first
"#,
        r#"run(first Transaction, second Transaction):
    selected: Pair(first: first, second: second).first
    selected.commit
    return
"#,
        r#"run(first Transaction, second Transaction):
    make(first, second).first.commit
    return
"#,
        r#"create Pair: return Pair(first: Transaction(state: 0), second: Transaction(state: 1))
run:
    create.first.commit
    return
"#,
        r#"run(first Transaction, second Transaction):
    Nested(value: Pair(first: first, second: second)).value.first.commit
    return
"#,
        r#"row(first own Transaction, second own Transaction) first Transaction, second Transaction:
    return first, second
run(first Transaction, second Transaction):
    row(first, second).first.commit
    return
"#,
    ] {
        expect_pending(body);
    }
}

#[test]
fn temporary_array_projections_preserve_unselected_elements() {
    for body in [
        r#"run(first Transaction, second Transaction):
    [first, second].[0].commit
    return
"#,
        r#"run(first Transaction, second Transaction, index is in 0 to 1):
    [first, second].[index].commit
    return
"#,
        r#"make_array(first own Transaction, second own Transaction) [Transaction; 2]:
    return [first, second]
run(first Transaction, second Transaction):
    make_array(first, second).[0].commit
    return
"#,
    ] {
        expect_pending(body);
    }
}

#[test]
fn temporary_projection_obligations_survive_branch_exits_and_joins() {
    for body in [
        r#"run(first Transaction, second Transaction, flag ref Byte):
    when flag is 0 then
        make(first, second).first.commit
        return
    finish(make(first, second))
    return
"#,
        r#"run(first Transaction, second Transaction, flag ref Byte):
    when flag is 0 then make(first, second).first.commit else finish(make(first, second))
    return
"#,
        r#"run(first Transaction, second Transaction, flag ref Byte) Transaction:
    return when flag is 0 then make(first, second).first else
        first.commit
        return second
"#,
    ] {
        expect_pending(body);
    }
}

#[test]
fn temporary_projection_obligations_are_checked_in_local_callables_and_loops() {
    for body in [
        r#"run(first Transaction, second Transaction):
    local(left Transaction, right Transaction):
        make(left, right).first.commit
        return
    local(first, second)
    return
"#,
        r#"run:
    for index in 0 to 2
        make(Transaction(state: 0), Transaction(state: 1)).first.commit
    return
"#,
    ] {
        expect_pending(body);
    }
}

#[test]
fn temporary_projections_can_transfer_all_resource_obligations() {
    for body in [
        r#"run(value Transaction):
    Single(value: value).value.commit
    return
"#,
        r#"run(value Transaction) Transaction: return Single(value: value).value
"#,
        r#"create Single: return Single(value: Transaction(state: 0))
run:
    create.value.commit
    return
"#,
        r#"run(value Transaction):
    [value].[0].commit
    return
"#,
        r#"run(value Transaction, state Byte):
    Mixed(value: value, state: state).value.commit
    return
"#,
        r#"row(value own Transaction, state Byte) selected Transaction, state Byte:
    return value, state
run(value Transaction, state Byte):
    row(value, state).selected.commit
    return
"#,
        r#"run(first Transaction, second Transaction):
    finish(Nested(value: make(first, second)).value)
    return
"#,
        r#"run(first Transaction, second Transaction, flag ref Byte):
    when flag is 0 then finish(make(first, second)) else finish(make(first, second))
    return
"#,
        r#"ArrayHolder has items [Transaction; 1]
run(value Transaction):
    ArrayHolder(items: [value]).items.[0].commit
    return
"#,
        r#"create Single: return Single(value: Transaction(state: 0))
run:
    for index in 0 to 2
        create.value.commit
    return
"#,
        r#"run:
    for index in 0 to 2
        [Transaction(state: 0)].[0].commit
    return
"#,
    ] {
        let flaws = check(body);
        assert!(flaws.is_empty(), "{body}: {flaws:?}");
    }
}

#[test]
fn temporary_diagnostics_identify_the_pending_field_at_the_branch_exit() {
    let body = r#"run(first Transaction, second Transaction, flag ref Byte):
    when flag is 0 then
        make(first, second).first.commit
        return
    finish(make(first, second))
    return
"#;
    let flaws = check(body);
    assert_eq!(flaws.len(), 1, "{flaws:?}");
    assert_eq!(
        flaws[0].code.slug().as_str(),
        "type-terminal-field-not-consumed"
    );
    assert_eq!(flaws[0].arg("name"), Some("temporary.second"));
    let source = format!("{CONTAINERS}{body}");
    let branch = &source[flaws[0].span.start()..flaws[0].span.end()];
    assert!(branch.contains("return"), "{branch}");
    assert!(flaws[0].span.end() <= source.find("    finish(make").unwrap());
}

#[test]
fn borrowed_destructured_fields_do_not_become_temporary_owners() {
    let flaws = check(
        r#"inspect(value ref Single) Byte:
    Single(value: tx) := value
    return tx.state
inspect_nested(value ref Nested) Byte:
    Nested(value: pair) := value
    return pair.first.state
"#,
    );
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn temporary_pattern_subjects_preserve_unmatched_payloads() {
    for body in [
        r#"wrap(value own Transaction) Maybe: return Present(value)
run(value Transaction):
    when wrap(value) is Absent then idle
    return
idle: return
"#,
        r#"wrap(value own Transaction) Maybe: return Present(value)
run(value Transaction):
    when wrap(value) is
        Absent then idle
        else idle
    return
idle: return
"#,
        r#"wrap(value own Transaction) Maybe: return Present(value)
run(value Transaction, flag ref Byte):
    when wrap(value) is Present(tx) and flag is 0 then tx.commit
    return
"#,
    ] {
        expect_pending(body);
    }
    for body in [
        r#"wrap(value own Transaction) Maybe: return Present(value)
run(value Transaction):
    when wrap(value) is Present(tx) then tx.commit
    return
"#,
        r#"wrap(value own Transaction) Maybe: return Present(value)
run(value Transaction):
    when wrap(value) is
        Present(tx) then tx.commit
        Absent then idle
    return
idle: return
"#,
    ] {
        let flaws = check(body);
        assert!(flaws.is_empty(), "{body}: {flaws:?}");
    }
}

#[test]
fn borrowing_a_temporary_projection_still_requires_a_binding() {
    let flaws = check(
        r#"inspect(value ref Transaction): return
run(value Transaction):
    inspect(Single(value: value).value)
    return
"#,
    );
    assert!(
        flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "type-reference-to-temporary"),
        "{flaws:?}"
    );
}
