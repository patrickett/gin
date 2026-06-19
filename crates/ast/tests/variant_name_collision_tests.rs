//! Tests that variant names are scoped to their owning union and do not
//! resolve to external types even when the names coincide.
//!
//! ```gin
//! Ordering is Less or Equal or Greater
//! Comparison is Ordered(Ordering) or Incomparable
//! --            ^^^^^^^ variant of Comparison, NOT the Ordering type
//! ```
//!
//! `Comparison.Ordered` is a variant name. `Ordering` is a type name.
//! A different union like `Comparison is Ordering or Incomparable` would
//! define `Comparison.Ordering` as a bare tag — also unrelated to the
//! `Ordering` type.

mod support;
use ast::source::SourceExt;
use parser::query::SourceParseExt;
use support::typed_file;

#[test]
fn variant_reference_detects_bare_tag_in_declare() {
    // `Ordering` in `Comparison is Ordering` is a bare-tag variant of
    // `Comparison`, NOT a reference to the `Ordering` type.
    // `union_variant_reference_at_byte` must return the variant name
    // so that goto-def resolves to the variant definition.
    let source = "\
Ordering is Less
         or Equal
         or Greater
Comparison is Ordering
           or Incomparable
";
    let output = source.parse_source_full();

    // Find byte position of `Ordering` as a variant inside Comparison
    let comparison_line = source.find("Comparison is ").expect("Comparison decl");
    let tag_start = comparison_line + "Comparison is ".len();

    let result = output
        .ast
        .union_variant_reference_at_byte(source, tag_start);
    assert_eq!(
        result,
        Some("Ordering".to_string()),
        "should detect Ordering as a variant reference at byte {tag_start}"
    );

    // Find byte position of `Ordering` in the type declaration at the top
    let decl_byte = source.find("Ordering is Less").expect("Ordering decl");
    let result_decl = output
        .ast
        .union_variant_reference_at_byte(source, decl_byte);
    assert_eq!(
        result_decl, None,
        "should NOT detect Ordering at type declaration site as a variant"
    );
}

#[test]
fn variant_reference_detects_generic_variant_name_in_declare() {
    // `Ordered` in `Comparison is Ordered(Ordering)` is a variant of
    // `Comparison` with a type reference payload.
    let source = "\
Ordering is Less
         or Equal
         or Greater
Comparison is Ordered(Ordering)
           or Incomparable
";
    let output = source.parse_source_full();

    // Find byte position of `Ordered` as a variant name
    let ordered_byte = source.find("Ordered(").expect("Ordered( in source");

    let result = output
        .ast
        .union_variant_reference_at_byte(source, ordered_byte);
    assert_eq!(
        result,
        Some("Ordered".to_string()),
        "should detect Ordered as a variant reference at byte {ordered_byte}"
    );

    // Find byte position of `Ordering` inside the payload — this should NOT
    // be a variant reference, since it's a type reference payload
    let ordering_payload = source.find("Ordering)").expect("Ordering in payload");
    let result_payload = output
        .ast
        .union_variant_reference_at_byte(source, ordering_payload);
    assert_eq!(
        result_payload, None,
        "should NOT detect Ordering in payload as a variant reference"
    );
}

#[test]
fn variant_ordered_is_not_ordering_type() {
    // When hovering `Ordered` in `Comparison is Ordered(Ordering)`,
    // it should show the variant shape — not redirect to the `Ordering` type.
    let source = "\
Ordering is Less
         or Equal
         or Greater
Comparison is Ordered(Ordering)
           or Incomparable
";
    let typed = typed_file(source);

    // Hover on `Ordered` in the Comparison declaration
    let ordered_byte = source.find("Ordered(").expect("Ordered( in source");
    let (line, character) = source.byte_offset_to_position(ordered_byte);
    let hover = typed
        .hover_at(source, line, character)
        .expect("hover on Ordered variant");

    // Should show the variant shape Ordered(Ordering), NOT the Ordering union
    assert!(
        hover.contains("Ordered(Ordering)"),
        "hover on Ordered should show the variant shape Ordered(Ordering), got: {hover:?}"
    );
    // Should NOT show the Ordering type declaration
    assert!(
        !hover.contains("Ordering is Less"),
        "hover on Ordered should NOT show the Ordering type declaration, got: {hover:?}"
    );
}

#[test]
fn ordering_type_hover_shows_union() {
    // When hovering `Ordering` in `Ordering is Less or Equal or Greater`,
    // it should show the `Ordering` union type.
    let source = "\
Ordering is Less
         or Equal
         or Greater
Comparison is Ordered(Ordering)
           or Incomparable
";
    let typed = typed_file(source);

    // Hover on `Ordering` in the type declaration
    let ordering_byte = source
        .find("Ordering is Less")
        .expect("Ordering declaration");
    let (line, character) = source.byte_offset_to_position(ordering_byte);
    let hover = typed
        .hover_at(source, line, character)
        .expect("hover on Ordering type");

    assert!(
        hover.contains("Ordering is Less"),
        "hover on Ordering type should show the union declaration, got: {hover:?}"
    );
}

#[test]
fn bare_tag_name_collision_is_separate_symbol() {
    // A union with a bare tag that happens to share a name with another type
    // should not resolve to that type.
    //
    // `BareComparison is Ordering or Incomparable` — `Ordering` here is a
    // variant of `BareComparison`, NOT the `Ordering` type.
    let source = "\
Ordering is Less
         or Equal
         or Greater
BareComparison is Ordering
               or Incomparable
";
    let typed = typed_file(source);

    // Hover on `Ordering` as a bare tag inside BareComparison
    let bare_start = source
        .find("BareComparison is ")
        .expect("BareComparison decl");
    let tag_start = bare_start + "BareComparison is ".len();
    let tag_text = &source[tag_start..];
    let ordered_pos_in_tag = tag_text.find("Ordering").expect("Ordering variant");
    let ordered_byte = tag_start + ordered_pos_in_tag;

    let (line, character) = source.byte_offset_to_position(ordered_byte);
    let hover = typed
        .hover_at(source, line, character)
        .expect("hover on bare Ordering tag");

    // Hover shows the bare tag `Ordering`, which is a variant of BareComparison.
    // It should NOT show the Ordering type declaration.
    assert!(
        !hover.contains("Ordering is Less"),
        "hover on bare Ordering tag should NOT show the Ordering type declaration, got: {hover:?}"
    );
}

#[test]
fn pattern_match_ordered_variant() {
    let source = "\
Ordering is Less
         or Equal
         or Greater
Comparison is Ordered(Ordering)
           or Incomparable

describe(c Comparison) String := when c is
    Ordered(o) then when o is
        Less    then 'less'
        Equal   then 'equal'
        Greater then 'greater'
    Incomparable then 'incomparable'
";
    let typed = typed_file(source);

    // Hover on `Ordered` in the when arm to verify it resolves to the variant
    let ordered_in_when = source.rfind("Ordered(o)").expect("Ordered(o) in when");
    let (line, character) = source.byte_offset_to_position(ordered_in_when);
    let hover = typed
        .hover_at(source, line, character)
        .expect("hover on Ordered in when arm");

    assert!(
        hover.contains("Ordered(Ordering)"),
        "hover on Ordered in pattern should show variant shape, got: {hover:?}"
    );
}

#[test]
fn pattern_match_incomparable_variant() {
    let source = "\
Ordering is Less
         or Equal
         or Greater
Comparison is Ordered(Ordering)
           or Incomparable

handle(c Comparison) String := when c is
    Ordered(o) then 'ordered'
    Incomparable then 'incomparable'
";
    let typed = typed_file(source);

    // Hover on `Incomparable` in the when arm
    let incomparable_in_when = source
        .rfind("Incomparable then")
        .expect("Incomparable in when");
    let (line, character) = source.byte_offset_to_position(incomparable_in_when);
    let hover = typed
        .hover_at(source, line, character)
        .expect("hover on Incomparable in when arm");

    // Hover shows the bare tag `Incomparable`
    assert!(
        hover.contains("Incomparable"),
        "hover on Incomparable in pattern should show variant, got: {hover:?}"
    );
    // Should NOT show the Ordering type
    assert!(
        !hover.contains("Ordering is Less"),
        "hover on Incomparable should NOT show the Ordering type, got: {hover:?}"
    );
}
