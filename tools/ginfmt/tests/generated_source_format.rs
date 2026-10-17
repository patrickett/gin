use ginfmt::format;
use parser::query::SourceParseExt;

mod common;

use common::assert_roundtrip;

fn assert_real_source_formats_cleanly(name: &str, source: &str) {
    assert!(!source.trim().is_empty(), "{name} fixture is empty");

    let formatted = format(source);
    assert!(
        !formatted.trim().is_empty(),
        "{name} formatted to empty output"
    );

    let reformatted = format(&formatted);
    assert_eq!(
        formatted, reformatted,
        "{name} is not idempotent after formatting"
    );

    let parsed = formatted.parse_source_full();
    assert!(
        parsed.symptoms.is_empty(),
        "{name} formatted output has parse symptoms: {:?}",
        parsed.symptoms
    );
}

const STRING_SOURCE: &str = "Byte is in 0 to 255\nString has ptr @Byte, len Byte\n";
const COMPARISON_SOURCE: &str = "Comparable has compare\n";
const ALLOCATOR_SOURCE: &str = "Allocation(element) has pointer @element, capacity Byte\n";

#[test]
fn preserves_standalone_comments_while_formatting() {
    let source = "-- leading\nValue is in 0 to 255\n\n-- callable boundary\nValue.new(value Value) Value: return value\n-- trailing\n";
    let formatted = format(source);
    assert!(formatted.contains("Value is in 0 to 255"));
    assert!(formatted.contains("-- callable boundary\nValue.new(value Value) Value: return value"));
    assert!(formatted.starts_with("-- leading\n"));
    assert!(formatted.ends_with("-- trailing\n"));
    assert_roundtrip(&formatted);
}

#[test]
fn parser_hints_do_not_prevent_formatting() {
    let source = "fetch(): return\nvalue:fetch()\n";
    assert!(!source.parse_source_full().symptoms.is_empty());
    let formatted = format(source);
    assert_ne!(formatted, source);
    assert_roundtrip(&formatted);
}

#[test]
fn block_bodies_have_no_trailing_spaces_or_extra_blank_lines() {
    let source = "first:\n    fetch()\n    return\nsecond:\n    fetch()\n    return\n";
    let formatted = format(source);
    assert!(formatted.lines().all(|line| line == line.trim_end()));
    assert!(!formatted.contains("\n\n\n"));
    assert!(!formatted.ends_with("\n\n"));
    assert_roundtrip(&formatted);
}

#[test]
fn preserves_sources_with_nested_or_inline_comments() {
    for source in [
        "value:1 -- inline\n",
        "main:\n    -- nested\n    value:1\n    return value\n",
    ] {
        assert_roundtrip(source);
    }
}

#[test]
fn formats_generated_string_fixture() {
    assert_real_source_formats_cleanly("generated string fixture", STRING_SOURCE);
}

#[test]
fn formats_generated_comparison_fixture() {
    assert_real_source_formats_cleanly("comparison fixture", COMPARISON_SOURCE);
}

#[test]
fn formats_real_program() {
    assert_real_source_formats_cleanly(
        "generated program fixture",
        "main:\n    value := 42\n    return value\n",
    );
}

#[test]
fn formats_long_parameter_lists_as_newline_separated_arguments() {
    let source = "signed_big_int_add_bits(lhs own SignedBigInt, rhs own SignedBigInt) SignedBigInt: return lhs\n";
    let formatted = format(source);

    assert_eq!(
        formatted,
        "signed_big_int_add_bits(\n    lhs own SignedBigInt\n    rhs own SignedBigInt\n) SignedBigInt: return lhs\n"
    );
}

#[test]
fn keeps_short_has_members_inline_without_alignment_padding() {
    assert_eq!(
        format(ALLOCATOR_SOURCE),
        "Allocation(element) has pointer @element, capacity Byte\n"
    );
}
