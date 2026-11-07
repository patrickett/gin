//! Files of one module get disjoint, file-naming span ids; file 0 matches plain parsing.
use parser::{
    ParseFailure, ParseLimit, ParseLimits, ParseOptions, parse_file, parse_strict,
    parse_strict_file,
};

const TEXT: &str = "Small is in 0 to 255\nidentity(value own Small) Small: value\n";

#[test]
fn file_zero_parses_identically_to_plain_parsing() {
    let module_file = parse_strict_file(TEXT, 0).unwrap();
    let plain = parse_strict(TEXT).unwrap();
    assert_eq!(module_file.items, plain.items);
    assert_eq!(module_file.comments, plain.comments);
    assert_eq!(module_file.text, plain.text);
}

#[test]
fn every_item_span_names_its_file_and_resolves() {
    for file_index in [0, 1, 5] {
        let file = parse_strict_file(TEXT, file_index).unwrap();
        assert!(!file.items.is_empty());
        for item in &file.items {
            assert_eq!(item.span_id.file(), file_index);
            assert!(!file.span_table.get(item.span_id).is_empty());
        }
    }
}

#[test]
fn ids_of_different_files_are_disjoint() {
    let first = parse_strict_file(TEXT, 0).unwrap();
    let second = parse_strict_file(TEXT, 1).unwrap();
    for item in &first.items {
        assert!(second.span_table.get(item.span_id).is_empty());
    }
    for item in &second.items {
        assert!(first.span_table.get(item.span_id).is_empty());
    }
}

#[test]
fn recovered_output_keeps_the_file_index() {
    let output = parse_file("broken Int(\nlater := 2\n", 3, ParseOptions::default()).unwrap();
    assert!(output.has_errors());
    for item in &output.file.items {
        assert_eq!(item.span_id.file(), 3);
    }
}

#[test]
#[should_panic]
fn file_indices_past_the_span_limit_are_programmer_errors() {
    let _ = parse_strict_file(TEXT, ast::SpanId::FILES_MAX);
}

#[test]
fn a_file_with_more_spans_than_its_id_range_is_a_limit_failure_not_a_panic() {
    let source = "a := 1\n".repeat(900_000);
    let options = ParseOptions {
        limits: ParseLimits {
            tokens_max: 10_000_000,
            ..ParseLimits::default()
        },
        ..ParseOptions::default()
    };
    let failure = parse_file(&source, 1, options).expect_err("span range exceeded");
    assert!(matches!(
        failure,
        ParseFailure::LimitExceeded {
            limit: ParseLimit::Spans,
            ..
        }
    ));
}
