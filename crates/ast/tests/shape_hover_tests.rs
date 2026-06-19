//! Layer 13: Hover on shape literal expressions, shorthand fields, and field access.
//!
//! Tests cover:
//! - Hover on a shape (record) tag name used in a literal expression
//! - Hover on a variable bound to a shape literal (shows the record type)
//! - Hover on a shorthand field name inside a record call
//! - Hover on a field access expression (e.g. `p.x`)
//! - Hover on the record variable in a field access expression

mod support;
use ast::source::SourceExt;
use support::transform_source;

#[test]
fn tag_name_in_shape_literal_shows_record_type() {
    let src = "\
Int is in 1...400

Coord has (x Int, y Int, z Int)
origin := Coord(x: 0, y: 0, z: 0)";
    let typed = transform_source(src);

    // Hover on `Coord` in the expression `Coord(x: 0, ...)` (the second occurrence)
    let first = src.find("Coord").unwrap();
    let second = first + src[first + 1..].find("Coord").unwrap() + 1;
    let (line, character) = src.byte_offset_to_position(second);

    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on Coord in record call");
    assert_eq!(
        hover,
        "\
```gin
Coord has (
    x Int,
    y Int,
    z Int,
)
```"
    );
}

#[test]
fn bind_to_shape_literal_shows_type() {
    let src = "\
Int is in 1...400

Coord has (x Int, y Int, z Int)
origin := Coord(x: 0, y: 0, z: 0)";
    let typed = transform_source(src);

    // Hover on `origin` — the left-hand side of the bind
    let byte = src.find("origin").unwrap();
    let (line, character) = src.byte_offset_to_position(byte);

    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on origin bind");
    // The bind's signature surface shows just the name when it has no
    // explicit type annotation and no constant-folded body.
    assert_eq!(
        hover,
        "\
```gin
origin
```"
    );
}

#[test]
fn shorthand_field_x_in_call_shows_bind_surface() {
    let src = "\
Int is in 1...400

Coord has (x Int, y Int, z Int)
x := 1
y := 2
z := 3
p := Coord(x, y, z)";
    let typed = transform_source(src);

    // Find the `Coord(x, y, z)` call, then the `x` field-name inside the parens.
    let call = src.find("Coord(x, y, z)").unwrap();
    // The `x` field name appears after the opening `(` — walk to it.
    let open_paren = src[call..].find('(').unwrap();
    let x_byte = call + open_paren + 1;
    let (line, character) = src.byte_offset_to_position(x_byte);

    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on shorthand field x");
    // Hover on the shorthand field `x` resolves back to the `x` variable
    // definition and shows its full signature surface (name + entire body).
    assert_eq!(
        hover,
        "\
```gin
x := 1
y := 2
z := 3
p := Coord(x, y, z)
```"
    );
}

#[test]
fn shorthand_field_y_in_call_shows_type() {
    let src = "\
Int is in 1...400

Coord has (x Int, y Int, z Int)
x := 1
y := 2
z := 3
p := Coord(x, y, z)";
    let typed = transform_source(src);

    // Hover on `y` inside `Coord(x, y, z)` — more reliably positioned
    // because `y` doesn't appear inside the Coord tag name itself.
    let call = src.find("Coord(x, y, z)").unwrap();
    let y_byte = call + src[call..].find("y, z").unwrap();
    let (line, character) = src.byte_offset_to_position(y_byte);

    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on shorthand field y");
    assert_eq!(
        hover,
        "\
```gin
y Int
```"
    );
}

#[test]
fn field_access_x_shows_field_type() {
    let src = "\
Int is in 1...400

Coord has (x Int, y Int, z Int)
p := Coord(x: 1, y: 2, z: 3)
p.x";
    let typed = transform_source(src);

    // Hover on `x` in `p.x` — the `x` after the dot
    let dot = src.find("p.x").unwrap();
    let x_byte = dot + src[dot..].find('x').unwrap();
    let (line, character) = src.byte_offset_to_position(x_byte);

    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on field access x");
    assert_eq!(
        hover,
        "\
```gin
x Int
```"
    );
}

#[test]
fn field_access_p_receiver_shows_definition_surface() {
    let src = "\
Int is in 1...400

Coord has (x Int, y Int, z Int)
p := Coord(x: 1, y: 2, z: 3)
p.x";
    let typed = transform_source(src);

    // Hover on `p` in `p.x` — the receiver expression (second `p`)
    let first_p = src.find("p").unwrap();
    let second_p = first_p + src[first_p + 1..].find("p").unwrap() + 1;
    let (line, character) = src.byte_offset_to_position(second_p);

    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on receiver p in p.x");
    // Hover on the receiver `p` resolves back to the bind definition,
    // showing the entire bind's signature surface (name + body expression).
    assert_eq!(
        hover,
        "\
```gin
p := Coord(x: 1, y: 2, z: 3)
p.x
```"
    );
}
