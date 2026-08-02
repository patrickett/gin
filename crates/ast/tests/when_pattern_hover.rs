//! Hover for `when` / `if` type-pattern sites and bound names in arm bodies.

mod support;
use ast::source::SourceExt;
use support::{transform_source, transform_with_marker_package};

#[test]
fn hover_wildcard_in_record_pattern() {
    let src = "\
Type is Primitive(width BigInt, signed Bool) or Record(name Str, fields List(NamedTy))
NamedTy has name Str, ty Type
List(x) has pointer Pointer(x), length BigInt
BigInt is in 0...1000
Str is in 0...255
Bool is True or False
Pointer(x) has addr BigInt

f(x Type) Bool := when x is
    Record(_, fields) then True
    else False
";
    let typed = transform_source(src);
    let arm = "    Record(_, fields) then True";
    let arm_start = src.find(arm).expect("when arm");
    let underscore = arm_start + arm.find('_').expect("_ in pattern");
    let (line, character) = src.byte_offset_to_position(underscore);
    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on _ in pattern");
    assert_eq!(
        hover, "```gin\n_: `in 0...255`\n```",
        "expected wildcard slot type, got: {hover}"
    );
}

#[test]
fn reflect_type_record_variant_map_resolves_forward_refs() {
    use internment::Intern;
    use typecheck::ty::Ty;

    // Self-contained: no gin_core fixture load (avoids multi-GB Reflectable shape materialization).
    let src = "\
NamedTy has name String, ty Type
List(x) has pointer Pointer(x), length BigInt
BigInt is in 0...1000
String has bytes List(Byte)
Byte is in 0...255
Bool is True or False
Pointer(x) has addr BigInt
Type is Primitive(width BigInt, signed Bool) or Record(name String, fields List(NamedTy))
";
    let typed = support::transform_source(src);
    let record = typed
        .variant_map
        .get(&Intern::from_ref("Record"))
        .and_then(|entries| {
            entries
                .iter()
                .find(|(union, _, _)| union.as_str() == "Type")
        })
        .expect("Type.Record in variant_map");
    let (_, _, fields) = record;
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].0.as_str(), "name");
    assert!(
        matches!(fields[0].1, Ty::Record { .. }),
        "name field should be String record, got {:?}",
        fields[0].1
    );
    assert_eq!(fields[1].0.as_str(), "fields");
    let Ty::Record { name, .. } = &fields[1].1 else {
        panic!("fields should be List(...), got {:?}", fields[1].1);
    };
    assert_eq!(name.as_str(), "List");
}

#[test]
fn record_pattern_fields_hover_matches_reflect_type() {
    let src = "\
NamedTy has name String, ty Type
List(x) has pointer Pointer(x), length BigInt
BigInt is in 0...1000
String has bytes List(Byte)
Byte is in 0...255
Bool is True or False
Pointer(x) has addr BigInt
Type is Primitive(width BigInt, signed Bool) or Record(name String, fields List(NamedTy))

f(x Type) Bool := when x is
    Record(_, fields) then True
    else False
";
    let typed = transform_source(src);
    let arm = "    Record(_, fields) then True";
    let arm_start = src.find(arm).expect("when arm");
    let fields_pos = arm_start + arm.find("fields").expect("fields binder");
    let (line, character) = src.byte_offset_to_position(fields_pos);
    let fields_hover = typed
        .hover_at(src, line, character)
        .expect("hover on fields");
    assert_eq!(
        fields_hover, "```gin\nfields List(NamedTy)\n```",
        "fields pattern binder hover"
    );

    let record_pos = arm_start + arm.find("Record").expect("Record");
    let (line, character) = src.byte_offset_to_position(record_pos);
    let record_hover = typed
        .hover_at(src, line, character)
        .expect("hover on Record");
    assert_eq!(
        record_hover,
        "```gin\nRecord(name bytes: pointer: addr: in 0...1000, length: in 0...1000, fields List)\n```",
        "expected Record(name String, fields List(NamedTy)), got: {record_hover}"
    );
}

#[test]
fn hover_record_variant_head_in_pattern() {
    let src = "\
Type is Primitive(width BigInt, signed Bool) or Record(name Str, fields List(NamedTy))
NamedTy has name Str, ty Type
List(x) has pointer Pointer(x), length BigInt
BigInt is in 0...1000
Str is in 0...255
Bool is True or False
Pointer(x) has addr BigInt

f(x Type) Bool := when x is
    Record(_, fields) then True
    else False
";
    let typed = transform_source(src);
    let record_byte = src.find("Record(_, fields)").unwrap();
    let (line, character) = src.byte_offset_to_position(record_byte);
    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on Record in pattern");
    assert_eq!(
        hover, "```gin\nRecord(name in 0...255, fields List)\n```",
        "expected Record hover with union type context, got: {hover}"
    );
}

#[test]
fn hover_false_in_ref_pattern_matches_bool_variant() {
    // Minimal: just need Ref variant + Bool for hover on `False` in pattern.
    let src = "\
Bool is True or False
BigInt is in 0...18446744073709551615

Type is Primitive(width BigInt, signed Bool)
     or Ref(inner Type, mutable Bool)

#auto
Copy has can_copy Bool: is_copy(Self)

is_copy(x Type) Bool := when x is
    Ref(_, False)       then True
    Ref(_, True)        then False
";
    let typed = transform_with_marker_package(src);
    let arm = "    Ref(_, False)       then True";
    let arm_start = src.find(arm).expect("Ref arm");
    let false_pos = arm_start + arm.find("False").expect("False in pattern");
    let (line, character) = src.byte_offset_to_position(false_pos);
    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on False in Ref pattern");
    assert_eq!(
        hover, "```gin\nFalse\n```",
        "expected Bool union variant hover, got: {hover}"
    );
}

#[test]
fn hover_true_in_then_body_shows_qualified_union_path() {
    // Minimal: just need Ref variant, Bool, and `True` in then-body for hover module prefix.
    let src = "\
Bool is True or False
BigInt is in 0...18446744073709551615

Type is Primitive(width BigInt, signed Bool)
     or Ref(inner Type, mutable Bool)

#auto
Copy has can_copy Bool: is_copy(Self)

is_copy(x Type) Bool := when x is
    Ref(_, False)       then True
";
    let typed = transform_with_marker_package(src);
    let mut index = typecheck::PackageSemanticIndex::from_typed_asts(&[&typed]);
    index.tag_module.insert(
        internment::Intern::from_ref("Bool"),
        "core.primitive".to_string(),
    );

    let arm = "    Ref(_, False)       then True";
    let arm_start = src.find(arm).expect("Ref/False arm");
    let true_pos = arm_start + arm.rfind("True").expect("True in then body");
    let (line, character) = src.byte_offset_to_position(true_pos);
    let hover = typed
        .hover_at_with_package(src, line, character, Some(&index))
        .expect("hover on True in then body");
    assert_eq!(
        hover.module_prefix.as_deref(),
        Some("core.primitive.Bool"),
        "variant hover should show qualified path to base union type"
    );
}

#[test]
fn hover_list_pattern_variables_not_bool() {
    let src = "\
List(x) has pointer Pointer(x), length BigInt\n\
BigInt is in 0...1000\n\
Bool is True or False\n\
Pointer(x) is @x\n\
NamedTy has name String, ty Type\n\
String has bytes List(Byte)\n\
Byte is in 0...255\n\
Type is Primitive(width BigInt, signed Bool) or Record(name String, fields List(NamedTy))\n\
\
all_copy(fields List(NamedTy)) Bool := when fields is\n\
    []                  then True\n\
    [f, ...rest]        then True\n\
";
    let typed = transform_source(src);

    // Hover on `f` in the list cons pattern `[f, ...rest]`
    let arm_byte = src.find("[f, ...rest]").unwrap();
    let f_byte = arm_byte + src[arm_byte..].find('f').unwrap();
    let (line, character) = src.byte_offset_to_position(f_byte);
    let f_hover = typed.hover_at(src, line, character);
    assert!(
        f_hover.is_some(),
        "hover on f in pattern should return Some"
    );
    if let Some(ref text) = f_hover {
        assert_eq!(text, "```gin\nf x\n```");
    }

    // Hover on `rest` in the cons pattern `[f, ...rest]`
    let rest_byte = arm_byte + src[arm_byte..].rfind("rest").unwrap();
    let (line, character) = src.byte_offset_to_position(rest_byte);
    let rest_hover = typed.hover_at(src, line, character);
    assert!(
        rest_hover.is_some(),
        "hover on rest in pattern should return Some"
    );
    if let Some(ref text) = rest_hover {
        assert_eq!(text, "```gin\nrest List(x)\n```");
    }
}
