use parser::query::SourceParseExt;

mod common;

use common::assert_roundtrip;

#[test]
fn formatting_keeps_overload_names_and_bodies() {
    let source = "choose(value A): return value\nchoose(value B): return value\n";
    let formatted = ginfmt::format(source);
    assert!(!formatted.contains('$'), "{formatted}");
    let parsed = formatted.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    assert_eq!(parsed.ast.defs.len(), 2);
    assert_roundtrip(&formatted);
}
