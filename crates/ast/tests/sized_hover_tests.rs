//! Hover on inline `sized.gin`-like source with explicit primitive definitions.

mod support;
use ast::source::SourceExt;
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

/// Primitive type definitions needed by `Type` and `Sized`.
const PRIMITIVES_SRC: &str = "\
BigInt is in 0...18446744073709551615
Bool is True or False
List(x) has (pointer Pointer(x), length BigInt)
Pointer(x) is @x
PointerSize is BigInt
String has (bytes List(BigInt))
ToString has (to_string String)
Happy has (value Bool)
";

/// The `Type` tag definition (equivalent to `reflect/type.gin`).
const TYPE_SRC: &str = "\
Type is Primitive(width BigInt, signed Bool)
     or Record(name String, fields List(NamedTy))
     or Union(name String, variants List(VariantShape))
     or Tuple(elems List(Type))
     or Ptr(inner Type)
     or Ref(inner Type, mutable Bool)
     or Array(elem Type, size BigInt)
     or Opaque(name String)

NamedTy has (name String, ty Type)
VariantShape has (name String, fields List(NamedTy))

Reflectable has (shape Type)
";

/// The `sized.gin` equivalent.
const SIZED_SRC: &str = "\
Size is Const(BigInt) or Dynamic

Sized has (size Size)

x.Sized(size: compute_size(x))

compute_size(x Type) Size := when x is
    Primitive(w, _)     then Const(w / 8)
    Ptr(_)              then Const(8)
    Ref(_, _)           then Const(8)
    Opaque(_)           then Dynamic
    Record(_, fields)   then sum_named(fields)
    Tuple(elems)        then sum_types(elems)
    Union(_, variants)  then union_size(variants)
    Array(elem, n)      then mul_size(compute_size(elem), n)

sum_named(fields List(NamedTy)) Size := when fields is
    []                  then Const(0)
    [f, ...rest]        then add(compute_size(f.ty), sum_named(rest))

sum_types(elems List(Type)) Size := when elems is
    []                  then Const(0)
    [t, ...rest]        then add(compute_size(t), sum_types(rest))

add(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(x + y)
                         else Dynamic

mul_size(size Size, n BigInt) Size := when size is
    Const(x) then Const(x * n)
             else Dynamic

union_disc(variants List(VariantShape)) Size := when variants is
    []           then Const(0)
    [_]          then Const(1)
                 else Const(2)

union_max_payload(variants List(VariantShape)) Size := when variants is
    []              then Const(0)
    [v, ...rest]    then max_size(sum_named(v.fields), union_max_payload(rest))

max_size(a Size, b Size) Size := when (a, b) is
    (Const(x), Const(y)) then Const(when x > y then x else y)
                         else Dynamic

union_size(variants List(VariantShape)) Size := add(union_disc(variants), union_max_payload(variants))
";

fn transform_sized_like_package() -> (typecheck::TypedFileAst, typecheck::PackageSemanticIndex) {
    // Build primitives as a single file (no `use` — all defined inline)
    let primitives = transform(
        &TokenCursor::parse_source(PRIMITIVES_SRC),
        FileId(0),
        &TransformCtx::new(),
    );
    let mut ctx = TransformCtx::from_typed_asts(&[&primitives]);
    let type_typed = transform(&TokenCursor::parse_source(TYPE_SRC), FileId(1), &ctx);
    ctx = TransformCtx::from_typed_asts(&[&primitives, &type_typed]);
    let sized = transform(&TokenCursor::parse_source(SIZED_SRC), FileId(2), &ctx);
    let index =
        typecheck::PackageSemanticIndex::from_typed_asts(&[&primitives, &type_typed, &sized]);
    (sized, index)
}

#[test]
fn hover_primitive_and_w_in_compute_size() {
    let src = SIZED_SRC;
    let (typed, index) = transform_sized_like_package();
    let arm = "    Primitive(w, _)     then Const(w / 8)";
    let arm_start = src.find(arm).expect("Primitive arm");
    let prim = arm_start + arm.find("Primitive").expect("Primitive");
    let (line, character) = src.byte_offset_to_position(prim);
    let hover = typed
        .hover_at_with_package(src, line, character, Some(&index))
        .expect("hover on Primitive")
        .markdown;
    assert_eq!(
        hover,
        "```gin\nPrimitive(width in 0...18446744073709551615, signed Bool)\n```"
    );
    let w_pos = arm_start + arm.find('w').expect("w binder");
    let (line, character) = src.byte_offset_to_position(w_pos);
    let hover = typed
        .hover_at_with_package(src, line, character, Some(&index))
        .expect("hover on w")
        .markdown;
    assert_eq!(hover, "```gin\nw in 0...18446744073709551615\n```");
}

#[test]
fn hover_compute_size_with_package_index_shows_type_not_i64() {
    let src = SIZED_SRC;
    let (typed, index) = transform_sized_like_package();
    let bind = typed
        .defs
        .values()
        .find(|b| b.name.as_str() == "compute_size")
        .expect("compute_size");
    let span = typed.span_table.get(bind.name_span);
    let (line, character) = src.byte_offset_to_position(span.start());
    let hover = typed
        .hover_at_with_package(src, line, character, Some(&index))
        .expect("hover on compute_size")
        .markdown;
    assert_eq!(hover, "```gin\ncompute_size(x Type) Size\n```");
}

#[test]
fn hover_record_pattern_fields_and_variant_head() {
    let src = SIZED_SRC;
    let typed = support::transform_with_full_marker_eval(src);
    let eval_typed = {
        let eval = support::load_marker_eval_ast_for_tests();
        typecheck::transform::transform_declare(
            &eval,
            typecheck::FileId(99),
            &typecheck::transform::TransformCtx::new(),
        )
    };
    let index = typecheck::PackageSemanticIndex::from_typed_asts(&[&eval_typed, &typed]);
    let arm = "    Record(_, fields)   then sum_named(fields)";
    let arm_start = src.find(arm).expect("Record arm");
    let fields_pos = arm_start + arm.find("fields").expect("fields");
    let (line, character) = src.byte_offset_to_position(fields_pos);
    let fields_hover = typed
        .hover_at_with_package(src, line, character, Some(&index))
        .expect("hover fields")
        .markdown;
    assert_eq!(fields_hover, "```gin\nfields List(NamedTy)\n```");
    let record_pos = arm_start + arm.find("Record").expect("Record");
    let (line, character) = src.byte_offset_to_position(record_pos);
    let record_hover = typed
        .hover_at_with_package(src, line, character, Some(&index))
        .expect("hover Record")
        .markdown;
    assert_eq!(
        record_hover,
        "```gin\nRecord(name bytes: pointer: in 0...18446744073709551615, length: in 0...18446744073709551615, fields List)\n```"
    );

    let sum_named_pos = arm_start + arm.find("sum_named").expect("sum_named");
    let (line, character) = src.byte_offset_to_position(sum_named_pos);
    let sum_hover = typed
        .hover_at_with_package(src, line, character, Some(&index))
        .expect("hover sum_named")
        .markdown;
    assert_eq!(
        sum_hover,
        "```gin\nsum_named(fields List(NamedTy)) Size\n```"
    );
}

#[test]
fn hover_compute_size_shows_type_param_not_i64() {
    let src = SIZED_SRC;
    let typed = support::transform_with_marker_package(src);
    let bind = typed
        .defs
        .values()
        .find(|b| b.name.as_str() == "compute_size")
        .expect("compute_size");
    let span = typed.span_table.get(bind.name_span);
    let (line, character) = src.byte_offset_to_position(span.start());
    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on compute_size");
    assert_eq!(hover, "```gin\ncompute_size(x Type) Size\n```");
}
