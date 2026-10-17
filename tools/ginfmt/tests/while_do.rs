use ginfmt::format;
use parser::query::SourceParseExt;

mod common;

use common::assert_roundtrip;

#[test]
fn while_do_survives_formatting_nested_bodies() {
    let source = "run:\n    while index < limit and not done do\n        while ready do\n            index::1\n        observe(index)\n    return index\n";
    let expected = "run:\n    while index < limit and not done do\n        while ready do\n            index :: 1\n        observe(index)\n    return index\n";
    assert!(source.parse_source_full().symptoms.is_empty());
    let formatted = format(source);
    assert_eq!(formatted, expected);
    assert!(formatted.parse_source_full().symptoms.is_empty());
    assert_roundtrip(&formatted);
}

#[test]
fn while_do_preserves_inline_comments() {
    let source = "run:\n    while index < limit do -- keep going\n        observe(index)\n    return index\n";
    assert!(source.parse_source_full().symptoms.is_empty());
    assert_roundtrip(source);
}
