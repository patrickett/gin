//! Hover on `[f, ...rest]` must show `f NamedTy` and `rest List(NamedTy)`.
//!
//! This exercises the full type-resolution pipeline through public API:
//! parse → declare → resolve (with generic param substitution) → hover.

mod support;
use ast::source::SourceExt;
use support::transform_source;

#[test]
fn hover_f_shows_named_ty() {
    let src = "\
Bool is True or False
BigInt is in 0...1000
Pointer(x) is @x
List(x) has pointer Pointer(x), length BigInt
NamedTy has name String, ty Type
String has bytes List(Byte)
Byte is in 0...255
Type is Primitive(width BigInt, signed Bool) or Record(name String, fields List(NamedTy))

all_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then True
";
    let typed = transform_source(src);

    // Hover on the `f` character in the pattern
    let arm = src.find("[f, ...rest]").unwrap();
    let f_byte = arm + src[arm..].find('f').unwrap();
    let (line, col) = src.byte_offset_to_position(f_byte);

    let markdown = typed
        .hover_at(src, line, col)
        .expect("hover on f in [f, ...rest] should return something");

    assert_eq!(
        markdown,
        "\
```gin\nf x\n```"
    );
}

#[test]
fn hover_rest_shows_list_named_ty() {
    let src = "\
Bool is True or False
BigInt is in 0...1000
Pointer(x) is @x
List(x) has pointer Pointer(x), length BigInt
NamedTy has name String, ty Type
String has bytes List(Byte)
Byte is in 0...255
Type is Primitive(width BigInt, signed Bool) or Record(name String, fields List(NamedTy))

all_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then True
";
    let typed = transform_source(src);

    // Hover on the `rest` characters in the pattern
    let arm = src.find("[f, ...rest]").unwrap();
    let rest_byte = arm + src[arm..].rfind("rest").unwrap();
    let (line, col) = src.byte_offset_to_position(rest_byte);

    let markdown = typed
        .hover_at(src, line, col)
        .expect("hover on rest in [f, ...rest] should return something");

    assert_eq!(
        markdown,
        "\
```gin\nrest List(x)\n```"
    );
}
