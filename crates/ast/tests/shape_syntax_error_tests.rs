//! Shape syntax error diagnostics — duplicate fields, unknown fields, missing
//! fields, and type mismatches in record-literal TagCalls (Layer 2 of the
//! shape syntax test plan).

mod support;

use support::transform_source;

#[test]
fn duplicate_field_name_reported() {
    let src = "\
Coord has x Int, y Int, z Int
p := Coord(x: 1, x: 2, y: 3, z: 4)
";
    let typed = transform_source(src);
    let flaws = typed.all_flaws();

    let has_duplicate_x = flaws
        .iter()
        .any(|(_, f)| f.code.slug() == "type-duplicate-field" && f.arg("name") == Some("x"));
    assert!(
        has_duplicate_x,
        "expected duplicate-field diagnostic for `x`, got: {flaws:?}"
    );
}

#[test]
fn unknown_field_name_reported() {
    let src = "\
Coord has x Int, y Int, z Int
p := Coord(x: 1, y: 2, w: 3)
";
    let typed = transform_source(src);
    let flaws = typed.all_flaws();

    let has_unknown_w = flaws.iter().any(|(_, f)| {
        (f.code.slug() == "type-unknown-field" && f.arg("name") == Some("w"))
            || f.message.contains("no field")
            || f.message.contains("w")
    });
    assert!(
        has_unknown_w,
        "expected unknown-field diagnostic for `w`, got: {flaws:?}"
    );
}

#[test]
fn missing_field_reported() {
    let src = "\
Coord has x Int, y Int, z Int
p := Coord(x: 1, y: 2)
";
    let typed = transform_source(src);
    let flaws = typed.all_flaws();

    let has_missing_z = flaws
        .iter()
        .any(|(_, f)| f.message.contains("missing") && f.message.contains("z"));
    assert!(
        has_missing_z,
        "expected missing-field diagnostic for `z`, got: {flaws:?}"
    );
}

#[test]
fn field_type_mismatch_reported() {
    let src = "\
Coord has x Int, y Int, z Int
p := Coord(x: \"hello\", y: 2, z: 3)
";
    let typed = transform_source(src);
    let flaws = typed.all_flaws();

    let has_type_mismatch = flaws.iter().any(|(_, f)| {
        f.code.slug() == "type-mismatch"
            || (f.message.contains("type") && f.message.contains("mismatch"))
    });
    assert!(
        has_type_mismatch,
        "expected type-mismatch diagnostic for field `x`, got: {flaws:?}"
    );
}
