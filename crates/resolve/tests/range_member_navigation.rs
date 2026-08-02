//! Go-to-definition tests for the has-body member model.
//!
//! These tests specify the expected goto-def behavior for:
//! - `self` keyword → receiver type
//! - `self.field` → field/property definition
//! - type parameters → type parameter declaration
//! - method params in bodies → param definition
//! - provided trait fields → field on receiver type
//!
//! Most are #[ignore]d because `cursor_definition` doesn't yet implement
//! self-keyword or has-body member navigation. When implementing those
//! features, remove #[ignore] and verify each case.

use parser::query::SourceParseExt;
use resolve::{CursorDefinition, cursor_definition};
use test_fixtures::TempPackage;

const RANGE_GIN: &str = include_str!("../../../modules/gin_core/primitive/range.gin");

fn definition_at(needle: &str) -> CursorDefinition {
    let pkg = TempPackage::new("range_member_navigation");
    let path = pkg.write("range.gin", RANGE_GIN);
    let output = RANGE_GIN.parse_source_full();

    let needle_without_cursor = needle.replace('|', "");
    let byte = RANGE_GIN
        .find(&needle_without_cursor)
        .expect("needle without cursor")
        + needle.find('|').unwrap_or(0);

    cursor_definition(&path, &output.ast, RANGE_GIN, byte, &|p| {
        resolve::ParsedFile::read(p)
    })
    .expect("definition")
}

fn definition_at_in(source: &str, needle: &str) -> CursorDefinition {
    let pkg = TempPackage::new("resolve_param_navigation");
    let path = pkg.write("fixture.gin", source);
    let output = source.parse_source_full();

    let needle_without_cursor = needle.replace('|', "");
    let byte = source
        .find(&needle_without_cursor)
        .expect("needle without cursor")
        + needle.find('|').unwrap_or(0);

    cursor_definition(&path, &output.ast, source, byte, &|p| {
        resolve::ParsedFile::read(p)
    })
    .expect("definition")
}

#[test]
fn goto_def_on_self_goes_to_receiver_type() {
    let def = definition_at("|self.start");
    let start = RANGE_GIN.find("Range(x) has").unwrap();
    assert_eq!(
        def,
        CursorDefinition::SameFile(start..start + "Range".len())
    );
}

#[test]
fn goto_def_on_self_member_goes_to_record_member() {
    let def = definition_at("self.|start");
    let start = RANGE_GIN.find("start x").unwrap();
    assert_eq!(
        def,
        CursorDefinition::SameFile(start..start + "start".len())
    );
}

#[test]
fn goto_def_on_receiver_type_param_uses_goes_to_receiver_type_param() {
    // `x` in `new(start x, end x)` should point to type param `x` in `Range(x)`.
    let def = definition_at("new(start |x");
    // Use "Range(x) has" to avoid matching the doc comment's `Range(x)`.
    let start = RANGE_GIN.find("Range(x) has").unwrap() + "Range(".len();
    assert_eq!(def, CursorDefinition::SameFile(start..start + "x".len()));
}

#[test]
fn goto_def_on_method_body_param_uses_goes_to_method_param() {
    // `start` in `Self(start, end)` body should point to param `start` in `new(start x, ...)`.
    let def = definition_at("Self(|start, end)");
    let new_decl = RANGE_GIN.find("new(start x, end x)").unwrap();
    let start = new_decl + "new(".len();
    assert_eq!(
        def,
        CursorDefinition::SameFile(start..start + "start".len())
    );

    // `end` in `Self(start, end)` body should point to param `end` in `new(..., end x)`.
    let def = definition_at("Self(start, |end)");
    let new_decl = RANGE_GIN.find("new(start x, end x)").unwrap();
    let end = new_decl + "new(start x, ".len();
    assert_eq!(def, CursorDefinition::SameFile(end..end + "end".len()));
}

#[test]
fn goto_def_on_provided_trait_self_field_refers_to_property() {
    // `self.start` in `Bounded.min: self.start` resolves to the `start x`
    // property on `Range(x)`.
    let def = definition_at("self.|start");
    let start = RANGE_GIN.find("start x").unwrap();
    assert_eq!(
        def,
        CursorDefinition::SameFile(start..start + "start".len())
    );
}

#[test]
fn goto_def_on_new_method_refers_to_has_body_method() {
    // The method name `new` in a call should resolve to the `new` declaration
    // inside the `Range(x) has` body.
    let def = definition_at("|new(start x");
    let start = RANGE_GIN.find("new(start x").unwrap();
    assert_eq!(def, CursorDefinition::SameFile(start..start + "new".len()));
}

#[test]
fn goto_def_on_provided_trait_self_refers_to_has_type() {
    // `self` in `Bounded.min: self.start` should point to `Range`.
    let def = definition_at("min: |self.start");
    let start = RANGE_GIN.find("Range(x) has").unwrap();
    assert_eq!(
        def,
        CursorDefinition::SameFile(start..start + "Range".len())
    );
}

#[test]
fn goto_def_on_type_param_annotating_property() {
    // `x` in property `start x` (where `x` is a type param) should resolve to
    // the type parameter `x` in `Range(x)`.
    let def = definition_at("start |x");
    // Use "Range(x) has" to avoid matching the doc comment's `Range(x)`.
    let start = RANGE_GIN.find("Range(x) has").unwrap() + "Range(".len();
    assert_eq!(def, CursorDefinition::SameFile(start..start + "x".len()));
}

#[test]
fn goto_def_type_parameters_inline_and_multiple() {
    let source = "Pair(left, right) has\n    first left\n    second right\n    new(left, right) Pair(left, right): Self(left, right)\n";

    let left_decl = source.find("new(left").unwrap() + "new(".len();
    let right_decl = source.find("new(left, ").unwrap() + "new(left, ".len();

    let left = definition_at_in(source, "Self(|left, right)");
    assert_eq!(left, CursorDefinition::SameFile(left_decl..left_decl + 4));

    let right = definition_at_in(source, "Self(left, |right)");
    assert_eq!(
        right,
        CursorDefinition::SameFile(right_decl..right_decl + 5)
    );
}

#[test]
fn goto_def_type_parameter_comma_and_newline_layout() {
    let source = "Vector(\n    x: Nat\n    y Int\n) has\n    data Array(x, y)\n";

    let x_param_decl = source.find("x: Nat").unwrap();
    let y_param_decl = source.find("y Int").unwrap();

    assert_eq!(
        definition_at_in(source, "Array(|x, y)"),
        CursorDefinition::SameFile(x_param_decl..x_param_decl + 1)
    );
    assert_eq!(
        definition_at_in(source, "Array(x, |y)"),
        CursorDefinition::SameFile(y_param_decl..y_param_decl + 1)
    );
}

#[test]
fn goto_def_type_parameter_annotated_function_and_member_body() {
    let source = "Pair(alpha Nat, beta) has\n    first alpha\n    second beta\n    new(alpha, beta) Pair(alpha, beta): Self(alpha, beta)\n";

    let alpha_decl = source.find("Pair(alpha").unwrap() + "Pair(".len();
    let beta_decl = source.find("Pair(alpha Nat, ").unwrap() + "Pair(alpha Nat, ".len();

    assert_eq!(
        definition_at_in(source, "new(|alpha, beta)"),
        CursorDefinition::SameFile(alpha_decl..alpha_decl + 5)
    );
    assert_eq!(
        definition_at_in(source, "new(alpha, |beta)"),
        CursorDefinition::SameFile(beta_decl..beta_decl + 4)
    );
}

#[test]
fn goto_def_type_parameters_in_function_signature_not_scanned() {
    let source = "Widget(a, b) has\n    a\n    b\n    get() Widget(a, b): Self(a, b)\n";

    let a_decl = source.find("Widget(a").unwrap() + "Widget(".len();
    let b_decl = source.find(", b").unwrap() + 2;

    assert_eq!(
        definition_at_in(source, "Self(|a, b)"),
        CursorDefinition::SameFile(a_decl..a_decl + 1)
    );
    assert_eq!(
        definition_at_in(source, "Self(a, |b)"),
        CursorDefinition::SameFile(b_decl..b_decl + 1)
    );
}

#[test]
#[ignore = "cursor_definition: Bounded is imported — resolves to use span, not external tag"]
fn goto_def_on_bounded_trait_refers_to_provided_trait() {
    // `Bounded` in `Range(x) has Bounded` should resolve to the `Bounded` tag.
    // Currently resolves to the import span (`use '../'.Bounded`) since the
    // tag is in an external file. Accept import span as valid.
    let def = definition_at("Range(x) has |Bounded");
    // The import is `'../'.Bounded` which spans bytes 4-17.
    let use_start = RANGE_GIN.find("'../'.Bounded").unwrap();
    assert_eq!(
        def,
        CursorDefinition::SameFile(use_start..use_start + "'../'.Bounded".len())
    );
}
