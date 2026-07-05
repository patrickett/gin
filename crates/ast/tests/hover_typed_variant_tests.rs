//! Typed AST hover for tags and union variants.

use ast::DeclareValue;
use ast::source::SourceExt;
use parser::query::SourceParseExt;

mod support;
use support::typed_file;

#[test]
fn hover_maybe_tag() {
    let source = "Maybe(x) is    --- Used to represent values that may or may not be present.\n    Some(x) or --- Has some value `x`\n    None       --- Has no value\n";
    let maybe_byte = source.find("Maybe").unwrap();
    let (line, character) = source.byte_offset_to_position(maybe_byte);
    let hover = typed_file(source)
        .hover_at(source, line, character)
        .expect("hover on `Maybe`");

    assert_eq!(
        hover,
        indoc::indoc! {r#"
            ```gin
            Maybe(x) is Some(x)
                     or None
            ```
            ---

            Used to represent values that may or may not be present.
        "#}
        .trim_end(),
        "hover on `Maybe`",
    );
}

#[test]
fn hover_some_variant() {
    let source = "Maybe(x) is    --- Used to represent values that may or may not be present.\n    Some(x) or --- Has some value `x`\n    None       --- Has no value\n";
    let some_byte = source.find("Some").unwrap();
    let (line, character) = source.byte_offset_to_position(some_byte);
    let hover = typed_file(source)
        .hover_at(source, line, character)
        .expect("hover on `Some`");

    assert_eq!(
        hover,
        indoc::indoc! {r#"
            ```gin
            Some(x)
            ```
            ---

            Has some value `x`
        "#}
        .trim_end(),
        "hover on `Some`",
    );
}

#[test]
fn hover_none_variant() {
    let source = "Maybe(x) is    --- Used to represent values that may or may not be present.\n    Some(x) or --- Has some value `x`\n    None       --- Has no value\n";
    let none_byte = source.find("None").unwrap();
    let (line, character) = source.byte_offset_to_position(none_byte);
    let hover = typed_file(source)
        .hover_at(source, line, character)
        .expect("hover on `None`");

    assert_eq!(
        hover,
        indoc::indoc! {r#"
            ```gin
            None
            ```
            ---

            Has no value
        "#}
        .trim_end(),
        "hover on `None`",
    );
}

#[test]
fn maybe_variants_have_doc_comments() {
    let source = "Maybe(x) is    --- Used to represent values that may or may not be present.\n    Some(x) or --- Has some value `x`\n    None       --- Has no value\n";
    let po = source.parse_source_full();

    let declare = po
        .ast
        .tags
        .get(&internment::Intern::<String>::from_ref("Maybe"))
        .expect("Maybe tag");

    let variants = match &declare.value {
        DeclareValue::Union { variants } => variants,
        other => panic!("expected Union, got {other:?}"),
    };

    assert_eq!(variants.len(), 2);
    let some_doc = match &variants[0] {
        ast::Variant::Local { doc_comment, .. } => doc_comment,
        ast::Variant::External { .. } => panic!("expected Local"),
    };
    assert_eq!(
        some_doc.as_ref().unwrap().value.as_str(),
        "Has some value `x`"
    );

    let none_doc = match &variants[1] {
        ast::Variant::Local { doc_comment, .. } => doc_comment,
        ast::Variant::External { .. } => panic!("expected Local"),
    };
    assert_eq!(none_doc.as_ref().unwrap().value.as_str(), "Has no value");
}

#[test]
fn hover_multi_variant_tag() {
    let source =
        "Color is\n    Red or --- Pure red\n    Green or --- Pure green\n    Blue --- Pure blue\n";
    let typed = typed_file(source);
    for variant in ["Red", "Green", "Blue"] {
        let byte = source.find(variant).unwrap();
        let (line, character) = source.byte_offset_to_position(byte);
        let hover = typed
            .hover_at(source, line, character)
            .unwrap_or_else(|| panic!("hover on `{variant}`"));

        let expected = match variant {
            "Red" => indoc::indoc! {r#"
                ```gin
                Red
                ```
                ---

                Pure red
            "#}
            .trim_end(),
            "Green" => indoc::indoc! {r#"
                ```gin
                Green
                ```
                ---

                Pure green
            "#}
            .trim_end(),
            "Blue" => indoc::indoc! {r#"
                ```gin
                Blue
                ```
                ---

                Pure blue
            "#}
            .trim_end(),
            other => panic!("unknown variant {other}"),
        };
        assert_eq!(hover, expected, "hover on `{variant}`");
    }
}
