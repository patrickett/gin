use parser::query::SourceParseExt;

mod common;

use common::assert_roundtrip;

#[test]
fn formatting_preserves_bracket_reads_and_writes() {
    let source = "read(values [in 0 to 255; count], index) : return values.[index]\nwrite(pointer @Word, value Word):\n    pointer.[0]:: value\n    return\n";
    let formatted = ginfmt::format(source);
    assert!(formatted.contains("values.[index]"), "{formatted}");
    assert!(formatted.contains("pointer.[0]:: value"), "{formatted}");
    let parsed = formatted.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    assert_roundtrip(&formatted);
}

#[test]
fn formatting_preserves_slice_borrows() {
    let source = "prefix(source ref [Byte; 4], n Length) ref [Byte; n]: return ref source.[0;n]\nupdate(source mut [Byte; 4], n Length) mut [Byte; n]: return mut source.[1;n]\n";
    let formatted = ginfmt::format(source);
    let parsed = formatted.parse_source_full();
    assert!(
        parsed.symptoms.is_empty(),
        "{formatted}: {:?}",
        parsed.symptoms
    );
    assert_roundtrip(&formatted);
}
