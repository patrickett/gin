mod support;

const RESOURCE: &str = r#"Byte is in 0 to 255
Byte.finish(self): return
Resource has
    value @Byte
    release(self): return
"#;

fn check(source: &str) -> Vec<String> {
    support::package_flaws(
        "ownership",
        &[
            ("resource", RESOURCE),
            (
                "client",
                &format!("use ownership.resource Byte, Resource\n{source}"),
            ),
        ],
    )
}

fn expect_move(source: &str) {
    let flaws = check(source);
    assert!(
        flaws.iter().any(|code| matches!(
            code.as_str(),
            "type-use-of-moved-value" | "type-use-after-move" | "type-use-of-maybe-moved-value"
        )),
        "{source}: {flaws:?}"
    );
}

fn expect_valid(source: &str) {
    let flaws = check(source);
    assert!(flaws.is_empty(), "{source}: {flaws:?}");
}

#[test]
fn captured_resources_are_consumed_at_local_calls() {
    for body in [
        "finish(0)\n    finish(0)",
        "finish(0)\n    value.release",
        "value.release\n    finish(0)",
    ] {
        expect_move(&format!(
            r#"run(value Resource):
    finish(ignored Byte): value.release
    {body}
    return
"#
        ));
    }
}

#[test]
fn one_local_call_can_discharge_a_captured_resource() {
    expect_valid(
        r#"run(value Resource):
    finish(ignored Byte): value.release
    finish(0)
    return
"#,
    );
}

#[test]
fn nested_local_calls_preserve_capture_consumption() {
    expect_move(
        r#"run(value Resource):
    finish(ignored Byte): value.release
    delegate(ignored Byte): finish(0)
    delegate(0)
    value.release
    return
"#,
    );
}

#[test]
fn captured_field_moves_preserve_unmoved_siblings() {
    let header = r#"Pair has
    first Resource
    second Resource
run(pair Pair):
    finish(ignored Byte): pair.first.release
    finish(0)
"#;
    expect_valid(&format!("{header}    pair.second.release\n    return\n"));
    expect_move(&format!(
        "{header}    finish(0)\n    pair.second.release\n    return\n"
    ));
}

#[test]
fn nested_local_parameters_can_transfer_resources() {
    expect_valid(
        r#"run(value Resource):
    finish(item own Resource):
        inner(owned own Resource): owned.release
        inner(item)
        return
    finish(value)
    return
"#,
    );
}

#[test]
fn conditional_local_consumption_does_not_discharge_all_paths() {
    expect_move(
        r#"run(value Resource, flag ref Byte):
    finish(ignored Byte):
        when flag is 0 then value.release
        return
    finish(0)
    value.release
    return
"#,
    );
}

#[test]
fn local_returns_keep_capture_effects() {
    expect_move(
        r#"run(value Resource, flag ref Byte):
    finish(ignored Byte):
        when flag is 0 then
            value.release
            return
        return
    finish(0)
    value.release
    return
"#,
    );
}

#[test]
fn returning_a_runtime_capture_transfers_it() {
    expect_move(
        r#"run(value is in 0 to 255):
    take(ignored Byte): return value
    take(0)
    return value
"#,
    );
}

#[test]
fn local_parameters_and_results_keep_terminal_obligations() {
    for local in [
        "forget(item Resource): return",
        "forget(item element): return",
        "forget(item Resource): 0",
        "forget(item Resource):\n        forward(item)\n        return",
    ] {
        let flaws = check(&format!(
            r#"forward(value own Resource) Resource: return value
run(value Resource):
    {local}
    forget(value)
    return
"#
        ));
        assert!(
            flaws
                .iter()
                .any(|code| code == "type-terminal-value-not-consumed"),
            "{local}: {flaws:?}"
        );
    }
}

#[test]
fn local_terminal_checks_exclude_outer_resources() {
    expect_valid(
        r#"run(first Resource, second Resource):
    finish(item Resource): item.release
    finish(first)
    second.release
    return
"#,
    );
}

#[test]
fn local_functions_can_return_owned_parameters() {
    expect_valid(
        r#"run(value Resource) Resource:
    forward(item own Resource) Resource: return item
    return forward(value)
"#,
    );
}

#[test]
fn local_borrow_observers_remain_callable_before_consumption() {
    expect_valid(
        r#"inspect(value ref Resource): return
run(value Resource):
    observe(ignored Byte): inspect(value)
    observe(0)
    observe(0)
    value.release
    return
"#,
    );
    expect_move(
        r#"inspect(value ref Resource): return
run(value Resource):
    observe(ignored Byte): inspect(value)
    value.release
    observe(0)
    return
"#,
    );
}

#[test]
fn local_consumption_invalidates_borrows_created_after_declaration() {
    let flaws = check(
        r#"inspect(value ref Resource): return
run(value Resource):
    finish(ignored Byte): value.release
    saved ref Resource: value
    finish(0)
    inspect(saved)
    return
"#,
    );
    assert!(
        flaws
            .iter()
            .any(|code| code.contains("invalidated-reference")),
        "{flaws:?}"
    );
}

#[test]
fn local_calls_cannot_read_invalidated_captured_borrows() {
    let flaws = check(
        r#"inspect(value ref Resource): return
run(value Resource):
    saved ref Resource: value
    observe(ignored Byte): inspect(saved)
    value.release
    observe(0)
    return
"#,
    );
    assert!(
        flaws
            .iter()
            .any(|code| code.contains("invalidated-reference")),
        "{flaws:?}"
    );
}

#[test]
fn repeated_arrays_cannot_duplicate_resources() {
    let flaws = check(
        r#"run(value Resource):
    values: [value; 2]
    values.[0].release
    values.[1].release
    value.release
    return
"#,
    );
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-sequence-repeat-seed-not-copyable"),
        "{flaws:?}"
    );
}

#[test]
fn generic_repeats_cannot_duplicate_resource_arguments() {
    for source in [
        r#"repeat(value element): return [value; 2]
run(value Resource): return repeat(value)
"#,
        r#"run(value Resource):
    repeat(item element): return [item; 2]
    return repeat(value)
"#,
    ] {
        let flaws = check(source);
        assert!(
            flaws
                .iter()
                .any(|code| code == "type-sequence-repeat-seed-not-copyable"),
            "{source}: {flaws:?}"
        );
    }
}

#[test]
fn repeated_arrays_cannot_duplicate_resources_with_trivial_payloads() {
    for fields in ["", "    value in 0 to 255\n"] {
        let source = format!(
            r#"Resource has
{fields}    release(self): return
repeat(value Resource): return [value; 2]
"#
        );
        let flaws = support::package_flaws("ownership", &[("resource", &source)]);
        assert!(
            flaws
                .iter()
                .any(|code| code == "type-sequence-repeat-seed-not-copyable"),
            "{source}: {flaws:?}"
        );
    }
}

#[test]
fn single_element_repeats_transfer_the_seed() {
    expect_move(
        r#"run(value Resource):
    values: [value; 1]
    values.[0].release
    value.release
    return
"#,
    );
    expect_valid(
        "run(value Resource):\n    values: [value; 1]\n    values.[0].release\n    return\n",
    );
}

#[test]
fn empty_repeats_cannot_discard_resource_temporaries() {
    let flaws = check(
        r#"forward(value own Resource) Resource: return value
run(value Resource):
    values: [forward(value); 0]
    return
"#,
    );
    assert!(
        flaws
            .iter()
            .any(|code| code == "type-terminal-value-not-consumed"),
        "{flaws:?}"
    );
    expect_valid("run(value Resource):\n    values: [value; 0]\n    value.release\n    return\n");
}

#[test]
fn symbolic_array_moves_conflict_with_overlapping_indices() {
    for body in [
        "values.[i].release\n    values.[0].release\n    values.[1].release",
        "values.[0].release\n    values.[i].release\n    values.[1].release",
    ] {
        expect_move(&format!(
            "run(values [Resource; 2], i is in 0 to 1):\n    {body}\n    return\n"
        ));
    }
}

#[test]
fn symbolic_array_reads_cannot_observe_consumed_elements() {
    expect_move(
        r#"inspect(value ref Resource): return
run(values [Resource; 2], i is in 0 to 1):
    values.[0].release
    inspect(values.[i])
    values.[1].release
    return
"#,
    );
}

#[test]
fn rebinding_an_index_cannot_reinterpret_an_earlier_move() {
    expect_move(
        r#"run(values [Resource; 2], source is in 0 to 1):
    index is in 0 to 1: source
    values.[index].release
    index:: 1
    values.[0].release
    values.[1].release
    return
"#,
    );
}

#[test]
fn mutating_an_index_field_cannot_reinterpret_an_earlier_move() {
    expect_move(
        r#"Cursor has
    index in 0 to 1
run(values [Resource; 2], cursor mut Cursor):
    values.[cursor.index].release
    cursor.index:: 1
    values.[0].release
    values.[1].release
    return
"#,
    );
}

#[test]
fn provably_disjoint_array_moves_preserve_siblings() {
    expect_valid(
        "run(values [Resource; 2]):\n    values.[0].release\n    values.[1].release\n    return\n",
    );
    let source = format!(
        r#"{RESOURCE}run(values [Resource; 2], i is in 0 to 1, j is in 0 to 1 and > i):
    values.[i].release
    values.[j].release
    return
"#
    );
    let flaws = support::package_flaws("ownership", &[("resource", &source)]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn same_module_bindings_transfer_resource_ownership() {
    for body in [
        "alias: value\n    value.release\n    alias.release",
        "alias: value\n    alias.release\n    value.release",
    ] {
        let source = format!("{RESOURCE}run(value Resource):\n    {body}\n    return\n");
        let flaws = support::package_flaws("ownership", &[("resource", &source)]);
        assert!(
            flaws.iter().any(|code| code == "type-use-of-moved-value"),
            "{flaws:?}"
        );
    }
    let source = format!(
        "{RESOURCE}run(value Resource):\n    alias: value\n    alias.release\n    return\n"
    );
    let flaws = support::package_flaws("ownership", &[("resource", &source)]);
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn every_return_checks_explicit_owned_parameters() {
    for parameter in ["value own Resource", "value own Byte"] {
        let operation = if parameter.ends_with("Resource") {
            "release"
        } else {
            "finish"
        };
        let source = format!(
            r#"{RESOURCE}run({parameter}, flag ref Byte):
    when flag is 0 then return
    value.{operation}
    return
"#
        );
        let flaws = support::package_flaws("ownership", &[("resource", &source)]);
        assert!(
            flaws
                .iter()
                .any(|code| code == "type-owned-param-not-consumed"),
            "{flaws:?}"
        );
        let source = format!(
            r#"{RESOURCE}run({parameter}, flag ref Byte):
    when flag is 0 then
        value.{operation}
        return
    value.{operation}
    return
"#
        );
        let flaws = support::package_flaws("ownership", &[("resource", &source)]);
        assert!(flaws.is_empty(), "{flaws:?}");
    }
}
