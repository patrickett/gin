use ginfmt::{Config, format};
use parser::query::SourceParseExt;

mod common;

use common::{assert_formatted, assert_roundtrip};

#[test]
fn owning_storage_fields_keep_their_modifier_and_align() {
    assert_formatted(
        "Storage(x) has\n    address own x\n    initialized Count(x)\n",
        "Storage(x) has\n    address     own x\n    initialized Count(x)\n",
        Config::default(),
    );
}

#[test]
fn aligns_argument_free_fields_before_method_bodies() {
    assert_formatted(
        "Reader has\n    data @Byte\n    position SignedBigInt\n    available SignedBigInt\n\n    new() Self: return Self(data: buffer(65536), position: 0, available: 0)\n",
        "Reader has\n    data      @Byte\n    position  SignedBigInt\n    available SignedBigInt\n\n    new() Self: return Self(data: buffer(65536), position: 0, available: 0)\n",
        Config::default(),
    );
}

#[test]
fn blank_lines_separate_alignment_groups() {
    assert_formatted(
        "Record has\n    a Int\n    bb Int\n\n    longer Int\n    c Int\n",
        "Record has\n    a  Int\n    bb Int\n\n    longer Int\n    c      Int\n",
        Config::default(),
    );
    assert_formatted(
        "Record has\n    a Int\n    bb Int\n\n    longer Int\n    c Int\n\n    read(self) Int: return self.a\n",
        "Record has\n    a  Int\n    bb Int\n\n    longer Int\n    c      Int\n\n    read(self) Int: return self.a\n",
        Config::default(),
    );
}

#[test]
fn documented_fields_preserve_their_docs_and_start_a_group() {
    assert_formatted(
        "Record has\n    a Int\n    bb Int\n    --- Stored count.\n    longer Int\n    c Int\n",
        "Record has\n    a  Int\n    bb Int\n    --- Stored count.\n    longer Int\n    c      Int\n",
        Config::default(),
    );
}

#[test]
fn generic_and_pointer_types_align_by_the_field_name() {
    assert_formatted(
        "Station has\n    name Vector(Byte)\n    minimum SignedBigInt\n    data @Byte\n",
        "Station has\n    name    Vector(Byte)\n    minimum SignedBigInt\n    data    @Byte\n",
        Config::default(),
    );
}

#[test]
fn multiline_types_separate_alignment_groups() {
    assert_formatted(
        "Record has\n    a Int\n    bb Int\n    items Vector(\n        Byte\n    )\n    c Int\n    longer Int\n\n    read(self) Int: return self.a\n",
        "Record has\n    a  Int\n    bb Int\n    items Vector(\n        Byte\n    )\n    c      Int\n    longer Int\n\n    read(self) Int: return self.a\n",
        Config::default(),
    );
}

#[test]
fn methods_and_computed_properties_separate_alignment_groups() {
    assert_formatted(
        "Record has\n    a Int\n    bb Int\n    computed Int: 0\n    c Int\n    longer Int\n    read(value Int) Int\n    d Int\n    ee Int\n",
        "Record has\n    a  Int\n    bb Int\n    computed Int: 0\n    c      Int\n    longer Int\n    read(value Int) Int\n    d  Int\n    ee Int\n",
        Config::default(),
    );
}

#[test]
fn alignment_respects_exact_width_and_padding_boundaries() {
    let source = "Record has\n    a LongerType\n    longer Int\n";
    for (width, padding, expected) in [
        (21, 5, "Record has\n    a      LongerType\n    longer Int\n"),
        (20, 5, source),
        (21, 4, source),
        (21, 0, source),
    ] {
        assert_formatted(
            source,
            expected,
            Config {
                max_line_width: width,
                max_type_alignment_padding: padding,
                ..Config::default()
            },
        );
    }
}

#[test]
fn private_blocks_align_separately_and_include_their_indent_in_width_limits() {
    let source =
        "Record has\n    #private\n        a LongerType\n        longer Int\n    visible Int\n";
    for (width, expected) in [
        (
            25,
            "Record has\n    #private\n        a      LongerType\n        longer Int\n    visible Int\n",
        ),
        (24, source),
    ] {
        assert_formatted(
            source,
            expected,
            Config {
                max_line_width: width,
                ..Config::default()
            },
        );
    }
}

#[test]
fn real_billion_row_program_aligns_fields() {
    let source = include_str!("../../../modules/one_billion_rows/main.gin");
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let formatted = format(source);
    assert!(
        formatted.contains(
            "    data      @Byte\n    position  SignedBigInt\n    available SignedBigInt\n"
        ),
        "{formatted}"
    );
    assert_roundtrip(&formatted);
    let reparsed = formatted.parse_source_full();
    assert!(reparsed.symptoms.is_empty());
    assert_eq!(reparsed.ast.defs.len(), parsed.ast.defs.len());
    assert!(formatted.contains("self.position :: SignedBigInt.replace"));
    assert!(formatted.contains("return ((value * 10 + fraction - 48) * sign) as SignedBigInt"));
}
