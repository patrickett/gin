//! Hover on `Tag is in lo...hi` must show the tag declaration, not unrelated exprs.

mod support;

use ast::source::SourceExt;
use parser::cursor::TokenCursor;
use support::transform_source;

#[test]
fn hover_on_signed_int_range_shows_tag_declare() {
    let src = "\
--- The 8-bit signed integer type.
SignedTinyInt is in -128...127
--- The 16-bit signed integer type.
SignedSmallInt is in -32768...32767
--- The 32-bit signed integer type.
SignedInt is in -2147483648...2147483647
--- The 64-bit signed integer type.
SignedBigInt is in -9223372036854775808...9223372036854775807
--- The 8-bit unsigned integer type.
TinyInt is in 0...255
--- The 16-bit unsigned integer type.
SmallInt is in 0...65535
--- The 32-bit unsigned integer type.
Int is in 0...4294967295
--- The 64-bit unsigned integer type.
BigInt is in 0...18446744073709551615
--- Alias for the 8-bit unsigned integer type.
Byte is TinyInt
";
    let typed = transform_source(src);

    let line = "SignedInt is in -2147483648...2147483647";
    let line_start = src.find(line).expect("SignedInt line");
    let range_pos = line_start + line.find("2147483647").expect("end of range");
    let (line, character) = src.byte_offset_to_position(range_pos);
    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on SignedInt range");

    assert_eq!(
        hover,
        "```gin\nSignedInt is in -2147483648...2147483647\n```\n---\n\nThe 32-bit signed integer type."
    );
}

#[test]
fn declare_span_covers_in_range_rhs() {
    let src = "SignedInt is in -1...1\n";
    let file = TokenCursor::parse_source(src);
    let decl = file
        .tags
        .get(&internment::Intern::from_ref("SignedInt"))
        .expect("tag");
    let span = file.span_table.get(decl.span);
    let rhs_pos = src.find("-1").expect("-1");
    assert!(
        span.contains(rhs_pos),
        "declare span should cover numeric range, got {span:?} for {src}"
    );
}
