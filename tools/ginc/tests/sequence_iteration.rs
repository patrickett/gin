mod common;
use common::{compile_and_run_exe, compile_exe};
use std::process::Command;

use ginc::cli::Profile;
use ginc::compile::{CompileFailure, CompileResult};
use test_fixtures::TempPackage;

const WORD: &str = "Word is in 0 to 255\nWord has\n    finish(self):\n        return\nreplace(previous Word, next Word) Word: return next\n";

fn run(name: &str, source: &str, expected: i32) {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new(name);
        package.write_flask(name);
        let input = package.write("main.gin", &format!("{WORD}{source}"));
        let output = compile_and_run_exe(
            input,
            &package.join("program"),
            profile,
            "",
            "run sequence traversal",
        );
        assert_eq!(output.status.code(), Some(expected), "{name} {profile:?}");
    }
}

#[test]
fn homogeneous_and_named_sequences_preserve_order_and_rebindings() {
    for (name, sequence) in [
        ("homogeneous", "[1, 2, 3]"),
        ("named", "[first: 1, second: 2, third: 3]"),
    ] {
        run(
            name,
            &format!(
                "#entry\nmain:\n    total Word: 0\n    for value in {sequence}\n        digit is in 0 to 255: value\n        previous is in 0 to 255: total\n        total:: replace(total, ((previous * 10 + digit) & 255) as Word)\n    return total\n"
            ),
            123,
        );
    }
}

#[test]
fn heterogeneous_sequences_call_each_elements_method() {
    run(
        "heterogeneous",
        "First has\n    score(self) Word: return 17\nSecond has\n    score(self) Word: return 25\n#entry\nmain:\n    total Word: 0\n    for value in [First(), Second()]\n        score: value.score\n        amount is in 0 to 255: score\n        previous is in 0 to 255: total\n        total:: replace(total, ((previous + amount) & 255) as Word)\n    return total\n",
        42,
    );
}

#[test]
fn sequence_source_is_evaluated_once_before_iteration() {
    run(
        "evaluate_source_once",
        "make(count mut Word) [Word; 2]:\n    previous is in 0 to 255: count\n    count:: replace(deref count, ((previous + 1) & 255) as Word)\n    return 10, 20\n#entry\nmain:\n    count Word: 0\n    for value in make(count)\n        value.finish\n    return count\n",
        1,
    );
}

#[test]
fn empty_sequence_skips_body() {
    run(
        "empty_sequence",
        "#entry\nmain:\n    total Word: 17\n    for value in []\n        total:: replace(total, 99)\n    return total\n",
        17,
    );
}

#[test]
fn iteration_bindings_preserve_outer_values_after_the_loop() {
    run(
        "sequence_scope",
        "#entry\nmain:\n    value Word: 42\n    for item in [1, 2]\n        saved: item\n    return value\n",
        42,
    );
}

#[test]
fn nested_sequences_are_elements_and_nested_traversal_works() {
    run(
        "nested_sequence",
        "#entry\nmain:\n    total Word: 0\n    for row in [[1, 2], [3]]\n        for value in row\n            amount is in 0 to 255: value\n            previous is in 0 to 255: total\n            total:: replace(total, ((previous + amount) & 255) as Word)\n    return total\n",
        6,
    );
}

#[test]
fn return_from_sequence_skips_later_elements() {
    run(
        "sequence_return",
        "choose Word:\n    for value in [1, 2, 3]\n        when value is 2 then\n            return 42\n    return 99\n#entry\nmain: choose\n",
        42,
    );
}

#[test]
fn bare_return_from_sequence_preserves_mutation() {
    run(
        "sequence_bare_return",
        "visit(total mut Word):\n    for value in [1, 2, 3]\n        when value is 2 then\n            total:: replace(deref total, 42)\n            return\n        total:: replace(deref total, 99)\n    return\n#entry\nmain:\n    total Word: 0\n    visit(total)\n    return total\n",
        42,
    );
}

#[test]
fn integer_ranges_choose_direction_and_include_the_endpoint() {
    for (name, range, expected) in [
        ("range_sequence", "2 to 5", 4),
        ("signed_range_sequence", "-2 to 1", 4),
        ("equal_range_sequence", "2 to 2", 1),
        ("descending_range_sequence", "5 to 2", 4),
        ("descending_signed_range_sequence", "1 to -2", 4),
        ("one_bit_range_sequence", "1 to 0", 2),
        ("ascending_one_bit_range_sequence", "0 to 1", 2),
        ("unsigned_sign_bit_sequence", "128 to 127", 2),
    ] {
        run(
            name,
            &format!(
                "count Word:\n    total Word: 0\n    for value in {range}\n        previous is in 0 to 255: total\n        total:: replace(total, ((previous + 1) & 255) as Word)\n    return total\n#entry\nmain: count\n"
            ),
            expected,
        );
    }
}

#[test]
fn integer_range_bounds_are_evaluated_once_before_the_body() {
    for (start, end, expected) in [(2, 5, 24), (5, 2, 24), (2, 2, 21)] {
        run(
            "range_bound_order",
            &format!(
                "lower(count mut Word) Word:\n    previous is in 0 to 255: count\n    count:: replace(deref count, ((previous + 1) & 255) as Word)\n    return {start}\nupper(count mut Word) Word:\n    previous is in 0 to 255: count\n    count:: replace(deref count, ((previous + 1) & 255) as Word)\n    return {end}\n#entry\nmain:\n    count Word: 0\n    total Word: 0\n    for value in lower(count) to upper(count)\n        previous is in 0 to 255: total\n        total:: replace(total, ((previous + 1) & 255) as Word)\n    calls is in 0 to 255: count\n    visits is in 0 to 255: total\n    return calls * 10 + visits\n"
            ),
            expected,
        );
    }
}

#[test]
fn integer_range_binder_is_independent_of_an_outer_constant() {
    run(
        "range_independent_constant",
        "#entry\nmain:\n    value Word: 42\n    total Word: 0\n    for item in 0 to 3\n        total:: replace(total, item as Word)\n    outer is in 0 to 255: value\n    last is in 0 to 255: total\n    return outer + last\n",
        45,
    );
}

#[test]
fn range_bound_mutation_is_visible_to_the_loop_body() {
    run(
        "range_bound_mutation",
        "lower(total mut Word) Word:\n    total:: replace(deref total, 1)\n    return 0\n#entry\nmain:\n    total Word: 0\n    for value in lower(total) to 2\n        previous is in 0 to 255: total\n        total:: replace(total, ((previous + 1) & 255) as Word)\n    return total\n",
        4,
    );
}

#[test]
fn inclusive_ranges_stop_at_machine_endpoints() {
    for (name, lower, upper) in [
        ("signed_max", "9223372036854775806", "9223372036854775807"),
        ("signed_min", "-9223372036854775808", "-9223372036854775807"),
        (
            "descending_signed_min",
            "-9223372036854775807",
            "-9223372036854775808",
        ),
        (
            "descending_signed_max",
            "9223372036854775807",
            "9223372036854775806",
        ),
        (
            "unsigned_max",
            "18446744073709551614",
            "18446744073709551615",
        ),
        (
            "descending_unsigned_max",
            "18446744073709551615",
            "18446744073709551614",
        ),
        (
            "unsigned_128_max",
            "340282366920938463463374607431768211454",
            "340282366920938463463374607431768211455",
        ),
        (
            "descending_unsigned_128_max",
            "340282366920938463463374607431768211455",
            "340282366920938463463374607431768211454",
        ),
    ] {
        run(
            name,
            &format!(
                "#entry\nmain:\n    total Word: 0\n    for value in {lower} to {upper}\n        previous is in Word: total\n        total:: replace(total, ((previous + 1) & 255) as Word)\n    return total\n"
            ),
            2,
        );
    }
}

#[test]
fn descending_range_preserves_order_rebindings_and_nested_loops() {
    run(
        "descending_order",
        "#entry\nmain:\n    total Word: 0\n    for value in 3 to 0\n        previous is in Word: total\n        total:: replace(total, ((previous * 4 + value) & 255) as Word)\n    return total\n",
        228,
    );
    run(
        "nested_descending_ranges",
        "#entry\nmain:\n    total Word: 0\n    for outer in 3 to 0\n        for inner in 2 to 0\n            previous is in Word: total\n            total:: replace(total, ((previous + outer + inner) & 255) as Word)\n    return total\n",
        30,
    );
}

#[test]
fn descending_range_can_return_early() {
    run(
        "descending_return",
        "choose Word:\n    for value in 1000000000 to 0\n        when value is 999999999 then\n            return 42\n    return 99\n#entry\nmain: choose\n",
        42,
    );
}

#[test]
fn runtime_bounds_select_both_directions_and_singleton_ranges() {
    for (start, end, expected) in [(3, 0, 228), (0, 3, 27), (2, 2, 2)] {
        run(
            "runtime_range_direction",
            &format!(
                "fold(start Word, end Word) Word:\n    total Word: 0\n    for value in start to end\n        previous is in Word: total\n        digit is in Word: value\n        total:: replace(total, ((previous * 4 + digit) & 255) as Word)\n    return total\n#entry\nmain: fold({start}, {end})\n"
            ),
            expected,
        );
    }
}

#[test]
fn mixed_integer_bound_domains_preserve_numeric_direction() {
    run(
        "mixed_range_domains",
        "choose(start is in 0 to 255, end is in -128 to 127) Word:\n    for value in start to end\n        when value = -1 then\n            return 42\n    return 99\n#entry\nmain: choose(1, -2)\n",
        42,
    );
}

const VIEW: &str = "Length is in 0 to 255\nLength has\n    finish(self):\n        return\nView(element) has\n    storage ref [element; length]\n    length Length\n    finish(self):\n        return\nprefix(source ref [Word; 4], length Length) View(Word):\n    data: ref source.[0; length]\n    return View(storage: data, length: length)\nread(value ref Word) Word: return deref value\n";

#[test]
fn borrowed_array_fields_index_the_referenced_array() {
    run(
        "borrowed_array_field",
        "Borrow has\n    storage ref [Word; 4]\n    finish(self):\n        return\npeek(view ref Borrow, index is in 0 to 3) Word: return view.storage.[index]\n#entry\nmain:\n    source [Word; 4]: [10, 20, 30, 40]\n    view: Borrow(storage: source)\n    return peek(view, 1)\n",
        20,
    );
}

#[test]
fn runtime_views_visit_only_the_visible_prefix() {
    for (length, expected) in [(0, 0), (1, 10), (2, 30), (4, 100)] {
        run(
            "visible_prefix",
            &format!(
                "{VIEW}run(length Length) Word:\n    source [Word; 4]: [10, 20, 30, 40]\n    view: prefix(source, length)\n    total Word: 0\n    for item in view.storage\n        previous is in Word: total\n        amount is in Word: read(item)\n        total:: replace(total, ((previous + amount) & 255) as Word)\n    return total\n#entry\nmain: run({length})\n"
            ),
            expected,
        );
    }
}

#[test]
fn runtime_view_return_skips_later_elements() {
    run(
        "view_return",
        &format!(
            "{VIEW}choose(view ref View(Word)) Word:\n    for value in view.storage\n        when read(value) is 20 then\n            return 42\n    return 99\n#entry\nmain:\n    source [Word; 4]: [10, 20, 30, 40]\n    view: prefix(source, 4)\n    return choose(view)\n"
        ),
        42,
    );
}

fn traps(name: &str, source: &str) {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new(name);
        package.write_flask(name);
        let input = package.write("main.gin", &format!("{WORD}{VIEW}{source}"));
        let output = package.join("program");
        let result = compile_exe(input, &output.clone(), profile);
        assert!(
            result.is_success() && !result.has_typecheck_diagnostics(),
            "{result:?}"
        );
        assert!(
            !Command::new(output).status().unwrap().success(),
            "{name} {profile:?}"
        );
    }
}

#[test]
fn runtime_view_index_traps_outside_the_visible_prefix() {
    traps(
        "view_index_bounds",
        "choose(length Length) Word:\n    source [Word; 4]: [10, 20, 30, 40]\n    view: prefix(source, length)\n    return view.storage.[3]\n#entry\nmain: choose(2)\n",
    );
}

#[test]
fn runtime_view_constructor_traps_if_length_exceeds_backing_array() {
    traps(
        "view_length_bounds",
        "choose(length Length) Word:\n    source [Word; 4]: [10, 20, 30, 40]\n    view: prefix(source, length)\n    return view.storage.[0]\n#entry\nmain: choose(5)\n",
    );
}

#[test]
fn dependent_views_borrow_array_storage_and_traverse_initialized_elements() {
    run(
        "pointer_view",
        "Length is in 0 to 255\nLength has\n    finish(self):\n        return\nWindow(element) has\n    storage ref [element; length]\n    length Length\n    finish(self):\n        return\nread(value ref Word) Word: return deref value\n#entry\nmain:\n    source [Word; 4]: [10, 20, 30, 40]\n    data: ref source.[0; 2]\n    view Window(Word): Window(storage: data, length: 2)\n    total Word: 0\n    for item in view.storage\n        previous is in Word: total\n        amount is in Word: read(item)\n        total:: replace(total, ((previous + amount) & 255) as Word)\n    return total\n",
        30,
    );
}

#[test]
fn core_array_and_list_views_use_the_sequence_primitive() {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("core_sequence_views");
        let core = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../modules/core")
            .canonicalize()
            .unwrap();
        package.copy_tree(&core, "core");
        package.write_flask_with_deps(
            "core_sequence_views",
            r#"core: "./core"
"#,
        );
        let input = package.write(
            "main.gin",
            r#"use core.slice Byte, ArrayView
use core.collection List
replace(previous Byte, next Byte) Byte: return next
read(value ref Byte) Byte: return deref value
#entry
main:
    array [Byte; 4]: [1, 2, 3, 4]
    prefix: ArrayView.new(array, 2)
    total Byte: 0
    for value in prefix.storage
        previous is in Byte: total
        amount is in Byte: read(value)
        total:: replace(total, ((previous + amount) & 255) as Byte)
    vector: when List.with_capacity(Byte, 2) is
        List(value) then value
        AllocFailure(failure) then
            failure.release
            return 1
    empty: vector.view
    for value in empty.data
        total:: replace(total, 99)
    when vector.length < vector.capacity then vector.push_reserved(10 as Byte)
    when vector.length < vector.capacity then vector.push_reserved(20 as Byte)
    view: vector.view
    for value in view.data
        previous is in Byte: total
        amount is in Byte: read(value)
        total:: replace(total, ((previous + amount) & 255) as Byte)
    vector.release
    return total
"#,
        );
        let output = package.join("program");
        let result = compile_exe(input, &output.clone(), profile);
        assert!(
            result.is_success() && !result.has_typecheck_diagnostics(),
            "{result:?}"
        );
        assert_eq!(Command::new(output).status().unwrap().code(), Some(33));
    }
}

#[test]
fn core_list_views_do_not_survive_storage_invalidation() {
    let core = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../modules/core")
        .canonicalize()
        .unwrap();
    for operation in ["values.clear", "values.reserve(100)", "values.release"] {
        let cleanup = if operation == "values.release" {
            ""
        } else {
            "    values.release\n"
        };
        for profile in [Profile::Debug, Profile::Release] {
            let package = TempPackage::new("core_view_invalidation");
            package.copy_tree(&core, "core");
            package.write_flask_with_deps(
                "core_view_invalidation",
                r#"core: "./core"
"#,
            );
            let input = package.write(
                "main.gin",
                &format!(
                    "use core.slice Byte\nuse core.collection List\n#entry\nmain:\n    values: when List.with_capacity(Byte, 2) is\n        List(value) then value\n        AllocFailure(failure) then\n            failure.release\n            return 1\n    when values.length < values.capacity then values.push_reserved(10 as Byte)\n    view: values.view\n    {operation}\n    result is in Byte: view.data.[0]\n{cleanup}    return result\n"
                ),
            );
            let output = package.join("program");
            let result = compile_exe(input, &output.clone(), profile);
            assert!(
                matches!(
                    result,
                    CompileResult::Failed(CompileFailure::TypecheckFailed)
                ),
                "{operation} {profile:?}: {result:?}"
            );
            assert!(!output.exists(), "{operation} {profile:?}");
        }
    }
}

#[test]
fn inclusive_unsigned_ranges_stop_at_the_maximum_endpoint() {
    run(
        "unsigned_range_max",
        "Wide is in 0 to 18446744073709551615\nWide has\n    finish(self):\n        return\nlower Wide: 18446744073709551614\nupper Wide: 18446744073709551615\n#entry\nmain:\n    total Word: 0\n    for value in lower to upper\n        previous is in Word: total\n        total:: replace(total, ((previous + 1) & 255) as Word)\n    return total\n",
        2,
    );
}

#[test]
fn runtime_view_sources_are_evaluated_once() {
    run(
        "view_evaluate_once",
        &format!(
            "{VIEW}make(source ref [Word; 4], calls mut Word) View(Word):\n    previous is in Word: calls\n    calls:: replace(deref calls, ((previous + 1) & 255) as Word)\n    return prefix(source, 2)\n#entry\nmain:\n    source [Word; 4]: [10, 20, 30, 40]\n    calls Word: 0\n    total Word: 0\n    for value in make(source, calls).storage\n        previous is in Word: total\n        amount is in Word: read(value)\n        total:: replace(total, ((previous + amount) & 255) as Word)\n    count is in Word: calls\n    sum is in Word: total\n    return count + sum\n"
        ),
        31,
    );
}

#[test]
fn runtime_view_binders_preserve_outer_constants() {
    run(
        "view_scope",
        &format!(
            "{VIEW}#entry\nmain:\n    source [Word; 4]: [10, 20, 30, 40]\n    view: prefix(source, 2)\n    value Word: 42\n    total Word: 0\n    for item in view.storage\n        previous is in Word: total\n        amount is in Word: read(item)\n        total:: replace(total, ((previous + amount) & 255) as Word)\n    outer is in Word: value\n    sum is in Word: total\n    return outer + sum\n"
        ),
        72,
    );
}

#[test]
fn inclusive_count_ranges_guard_zero_before_unsigned_subtraction() {
    for (length, expected) in [(0, 0), (1, 1), (4, 4)] {
        run(
            "count_range",
            &format!(
                "Length is in 0 to 18446744073709551615\nLength has\n    finish(self):\n        return\ncount(length Length) Word:\n    count is in Length: length\n    total Word: 0\n    when count is 0 then\n        return total\n    for index in 0 to count - 1\n        previous is in Word: total\n        total:: replace(total, ((previous + 1) & 255) as Word)\n    return total\n#entry\nmain: count({length})\n"
            ),
            expected,
        );
    }
}

#[test]
fn runtime_view_constructor_checks_the_backing_extent() {
    traps(
        "view_literal_extent",
        "build(source ref [Word; 4], length Length) View(Word):\n    data: ref source.[0; length]\n    return View(storage: data, length: length)\n#entry\nmain:\n    source [Word; 4]: [10, 20, 30, 40]\n    view: build(source, 5)\n    return read(view.storage.[0])\n",
    );
}

const MUTABLE_VIEW: &str = "Length is in 0 to 255\nLength has\n    finish(self):\n        return\nMutableView(element) has\n    storage mut [element; length]\n    length Length\n    finish(self):\n        return\nmutable_prefix(source mut [Word; 4], length Length) MutableView(Word):\n    data: mut source.[0; length]\n    return MutableView(storage: data, length: length)\nset(value mut Word):\n    value:: replace(deref value, 42)\n    return\n";

#[test]
fn mutable_runtime_views_update_only_the_visible_prefix() {
    for (length, first, last) in [(0, 10, 40), (1, 42, 40), (2, 42, 40), (4, 42, 42)] {
        run(
            "mutable_prefix",
            &format!(
                "{MUTABLE_VIEW}visit(length Length) Word:\n    source [Word; 4]: [10, 20, 30, 40]\n    view: mutable_prefix(source, length)\n    for value in view.storage\n        set(value)\n    view.finish\n    return when source.[0] is {first} and source.[3] is {last} then 0 else 1\n#entry\nmain: visit({length})\n"
            ),
            0,
        );
    }
}

#[test]
fn mutable_runtime_view_writes_check_the_visible_length() {
    traps(
        "mutable_view_bound",
        &format!(
            "{MUTABLE_VIEW}write(view mut MutableView(Word), index Length):\n    view.storage.[index]:: replace(view.storage.[index], 42)\n    return\n#entry\nmain:\n    source [Word; 4]: [10, 20, 30, 40]\n    view: mutable_prefix(source, 2)\n    write(view, 2)\n    view.finish\n    return 0\n"
        ),
    );
}

#[test]
fn core_mutable_array_and_list_views_update_their_storage() {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("core_mutable_sequence_views");
        let core = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../modules/core")
            .canonicalize()
            .unwrap();
        package.copy_tree(&core, "core");
        package.write_flask_with_deps(
            "core_mutable_sequence_views",
            r#"core: "./core"
"#,
        );
        let input = package.write("main.gin", r#"use core.slice Byte, ArrayMutView
use core.collection List
increment(value mut Byte):
    previous is in Byte: value
    value:: Byte.replace(deref value, ((previous + 1) & 255) as Byte)
    return
#entry
main:
    array [Byte; 4]: [10, 20, 30, 40]
    prefix: ArrayMutView.new(array, 2)
    prefix.storage.[0]:: Byte.replace(prefix.storage.[0], 9)
    for value in prefix.storage
        increment(value)
    vector: when List.with_capacity(Byte, 2) is
        List(value) then value
        AllocFailure(failure) then
            failure.release
            return 1
    empty: vector.view_mut
    for value in empty.data
        increment(value)
    when vector.length < vector.capacity then vector.push_reserved(10 as Byte)
    when vector.length < vector.capacity then vector.push_reserved(20 as Byte)
    view: vector.view_mut
    for value in view.data
        increment(value)
    shared: view.as_ref
    borrowed_first is in Byte: shared.data.[0]
    observed: vector.view
    first is in Byte: observed.data.[0]
    second is in Byte: observed.data.[1]
    vector.release
    return when array.[0] is 10 and array.[1] is 21 and array.[2] is 30 and first is 11 and second is 21 and borrowed_first is 11 then 0 else 1
"#);
        let output = package.join("program");
        let result = compile_exe(input, &output.clone(), profile);
        assert!(
            result.is_success() && !result.has_typecheck_diagnostics(),
            "{profile:?}: {result:?}"
        );
        assert_eq!(Command::new(output).status().unwrap().code(), Some(0));
    }
}

#[test]
fn directly_constructed_mutable_views_preserve_writes_and_early_returns() {
    run(
        "mutable_view_return",
        &format!(
            "{MUTABLE_VIEW}visit(view mut MutableView(Word)):\n    for value in view.storage\n        when deref value is 10 then\n            set(value)\n            return\n    return\n#entry\nmain:\n    source [Word; 4]: [10, 20, 30, 40]\n    view MutableView(Word): MutableView(storage: source, length: 4)\n    visit(view)\n    view.finish\n    return when source.[0] is 42 and source.[1] is 20 then 0 else 1\n"
        ),
        0,
    );
    run(
        "mutable_view_direct_write",
        &format!(
            "{MUTABLE_VIEW}#entry\nmain:\n    source [Word; 4]: [10, 20, 30, 40]\n    view MutableView(Word): mutable_prefix(source, 2)\n    view.storage.[1]:: replace(view.storage.[1], 42)\n    view.finish\n    return when source.[1] is 42 and source.[2] is 30 then 0 else 1\n"
        ),
        0,
    );
}
