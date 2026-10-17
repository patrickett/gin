use parser::query::SourceParseExt;

mod common;

use common::{assert_formatted_at_width, assert_roundtrip};

#[test]
fn signatures_use_their_actual_width() {
    let source = "Pool has\n    reserve(self mut, element, count Count(element)) Attempt(element)\n    reclaim(self mut, element, allocation StoreBlock(element))\n    release(self)\n";
    assert_formatted_at_width(source, source, 80);
}

#[test]
fn return_type_counts_toward_signature_width() {
    assert_formatted_at_width(
        "Reader has read(self ref, count Int) VeryLongResultType\n",
        "Reader has\n    read(\n        self ref\n        count Int\n    ) VeryLongResultType\n",
        40,
    );
}

#[test]
fn short_multiline_unions_stay_multiline() {
    for source in [
        "Attempt(element) is Block(element)\n    or Failed(Error)\n",
        "Attempt(element) is\n    Block(element)\n    or Failed(Error)\n",
    ] {
        assert_formatted_at_width(source, source, 120);
    }
}

#[test]
fn short_multiline_parameters_stay_multiline() {
    for source in [
        "Reader has\n    read(\n        self ref\n        count Int\n    ) Int\n",
        "Reader has\n    read(\n        self\n        count Int\n    ) Int\n",
        "Pair(\n    left\n    right\n) has\n    first  left\n    second right\n",
        "add(\n    left Int\n    right Int\n) Int: return left + right\n",
    ] {
        assert_formatted_at_width(source, source, 120);
    }
}

#[test]
fn compact_unions_and_parameters_stay_compact() {
    let source = "Attempt(element) is Block(element) or Failed(Error)\nPair(left, right) has first left, second right\n";
    assert_formatted_at_width(source, source, 120);
}

#[test]
fn multiline_type_parameters_do_not_force_multiline_variants() {
    let source = "Attempt(\n    element\n) is Block(element) or Failed(Error)\n";
    assert_formatted_at_width(source, source, 120);
}

#[test]
fn short_multiline_members_stay_multiline() {
    let source = "Box(element) has\n    value element\n";
    assert_formatted_at_width(source, source, 120);
}

#[test]
fn signature_wraps_only_when_it_exceeds_the_width() {
    let source = "Reader has\n    read(self ref, count Int) Int\n";
    let width = "    read(self ref, count Int) Int".len();
    assert_formatted_at_width(source, source, width);
    assert_formatted_at_width(
        source,
        "Reader has\n    read(\n        self ref\n        count Int\n    ) Int\n",
        width - 1,
    );
}

#[test]
fn constructor_binds_wrap_without_commas() {
    assert_formatted_at_width(
        "a Allocation(x): Allocation(address: typed_pointer, capacity: c, initialized: 0 as Count(x))\n",
        "a Allocation(x): Allocation(\n    address: typed_pointer\n    capacity: c\n    initialized: 0 as Count(x)\n)\n",
        80,
    );
}

#[test]
fn method_body_binds_wrap_with_body_indentation() {
    assert_formatted_at_width(
        "Pool has\n    reserve(self mut, c Count(x)) Allocation(x):\n        a Allocation(x): Allocation(address: typed_pointer, capacity: c, initialized: 0 as Count(x))\n        return a\n",
        "Pool has\n    reserve(self mut, c Count(x)) Allocation(x):\n        a Allocation(x): Allocation(\n            address: typed_pointer\n            capacity: c\n            initialized: 0 as Count(x)\n        )\n        return a\n",
        80,
    );
}

#[test]
fn multiline_call_binds_stay_multiline() {
    let source = "a Allocation(x): Allocation(\n    capacity: c\n    initialized: 0\n)\n";
    assert_formatted_at_width(source, source, 120);
}

#[test]
fn call_binds_use_the_entire_line_width() {
    let source = "a Allocation(x): Allocation(capacity: c, initialized: 0)\n";
    let width = source.trim_end().len();
    assert_formatted_at_width(source, source, width);
    assert_formatted_at_width(
        source,
        "a Allocation(x): Allocation(\n    capacity: c\n    initialized: 0\n)\n",
        width - 1,
    );
}

#[test]
fn function_call_binds_wrap_without_commas() {
    assert_formatted_at_width(
        "result: storage.reserve(capacity: count, alignment: required_alignment)\n",
        "result: storage.reserve(\n    capacity: count\n    alignment: required_alignment\n)\n",
        60,
    );
}

#[test]
fn when_then_else_align_in_one_column() {
    assert_formatted_at_width(
        "size: when length * width = 0 then 1 else length * width\n",
        "size: when length * width = 0\n      then 1\n      else length * width\n",
        80,
    );
}

#[test]
fn method_body_when_then_else_align_in_one_column() {
    assert_formatted_at_width(
        "Pool has\n    reserve(self mut):\n        size: when length * width = 0 then 1 else length * width\n        return size\n",
        "Pool has\n    reserve(self mut):\n        size: when length * width = 0\n              then 1\n              else length * width\n        return size\n",
        80,
    );
}

#[test]
fn nested_calls_wrap_at_each_argument_indent() {
    assert_formatted_at_width(
        "result: Outer(value: Inner(first: first_value, second: second_value), other: other_value)\n",
        "result: Outer(\n    value: Inner(\n        first: first_value\n        second: second_value\n    )\n    other: other_value\n)\n",
        50,
    );
}

#[test]
fn grouped_constructor_binds_keep_parentheses() {
    assert_formatted_at_width(
        "result: (Outer(first: first_value, second: second_value))\n",
        "result: (Outer(\n    first: first_value\n    second: second_value\n))\n",
        50,
    );
}

#[test]
fn nested_when_branches_stay_separate() {
    assert_formatted_at_width(
        "result: when first is True then 1 else when second is True then 2 else 3\n",
        "result: when first is True\n        then 1\n        else when second is True\n             then 2\n             else 3\n",
        80,
    );
}

#[test]
fn composed_and_negated_conditions_use_multiline_branches() {
    for condition in ["first is True and second is True", "not first is True"] {
        assert_formatted_at_width(
            &format!("result: when {condition} then 1 else 0\n"),
            &format!("result: when {condition}\n        then 1\n        else 0\n"),
            80,
        );
    }
}

#[test]
fn global_allocator_is_formatted_within_eighty_columns() {
    let source = include_str!("../../../modules/core/alloc/global.gin");
    assert!(
        source.parse_source_full().symptoms.is_empty(),
        "{:?}",
        source.parse_source_full().symptoms
    );
    let formatted = ginfmt::format(source);
    assert!(
        formatted.lines().all(|line| line.chars().count() <= 80),
        "{formatted}"
    );
    assert!(formatted.contains("Allocation(\n        "), "{formatted}");
    assert!(formatted.parse_source_full().symptoms.is_empty());
    assert_roundtrip(&formatted);
}

#[test]
fn when_alignment_uses_the_maximum_width_boundary() {
    let prefix = "long_result_name: ";
    let fallback = "very_long_fallback_value";
    let padding = " ".repeat(prefix.len());
    let width = prefix.len() + "else ".len() + fallback.len();
    for (body, otherwise) in [("1", fallback), (fallback, "0")] {
        let source = format!("{prefix}when ready is True then {body} else {otherwise}\n");
        let aligned = format!(
            "{prefix}when ready is True\n{padding}then {body}\n{padding}else {otherwise}\n"
        );
        assert_formatted_at_width(&source, &aligned, width);
        assert_formatted_at_width(
            &source,
            &format!("{prefix}when ready is True\n    then {body}\n    else {otherwise}\n"),
            width - 1,
        );
    }
}

#[test]
fn when_width_fallback_does_not_increase_indentation() {
    assert_formatted_at_width(
        "r: when ready is True then 1 else very_long_fallback_value\n",
        "r: when ready is True\nthen 1\nelse very_long_fallback_value\n",
        30,
    );
}

#[test]
fn when_alignment_counts_indentation_in_method_bodies() {
    let source = "Pool has\n    reserve(self mut):\n        result: when ready is True then 1 else very_long_fallback_value\n        return result\n";
    assert_formatted_at_width(
        source,
        "Pool has\n    reserve(self mut):\n        result: when ready is True\n                then 1\n                else very_long_fallback_value\n        return result\n",
        45,
    );
    assert_formatted_at_width(
        source,
        "Pool has\n    reserve(self mut):\n        result: when ready is True\n            then 1\n            else very_long_fallback_value\n        return result\n",
        44,
    );
}

#[test]
fn value_blocks_and_explicit_function_returns_survive_formatting() {
    for source in [
        "value:\n    adjusted: 7\n    adjusted\n",
        "choose(captured Int) Int:\n    value: when\n        captured < 0 then\n            adjusted: -captured\n            adjusted\n        else\n            captured\n    return value\n",
        "choose(captured Int) Int: return captured\n",
    ] {
        assert_formatted_at_width(source, source, 120);
    }
}

#[test]
fn nested_is_blocks_keep_guards_bodies_and_outer_alternatives_separate() {
    let source = "choose(value) Int:\n    return when value is\n        Some(x) and x is\n            Left(y) and y > 0 then\n                adjusted: y + 1\n                adjusted\n            Right(y) then y\n        Some(_) then 0\n        None then -1\n";
    assert_formatted_at_width(source, source, 120);
}
