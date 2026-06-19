//! Hover on `copy.gin`-like inline source — signatures and `when` patterns.

mod support;
use ast::source::SourceExt;
use std::sync::OnceLock;
use support::transform_with_marker_package;

fn copy_src() -> &'static str {
    // Types resolve from the marker package fixture; only need the is_copy
    // definitions. Type annotations on helper params are kept where tests
    // search for them by substring.
    "\
Copy has (can_copy Bool)
x.Copy(can_copy: is_copy(x))

is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
    Ref(_, False)       then True
    Ref(_, True)        then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)
    Union(_, variants)  then all_variants_copy(variants)

all_named_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_variants_copy(variants List(VariantShape)) Bool := when variants is
    []                  then True
    [v, ...rest]        then all_named_copy(v.fields) and all_variants_copy(rest)
"
}

fn copy_typed_marker_package() -> &'static typecheck::TypedFileAst {
    static TYPED: OnceLock<typecheck::TypedFileAst> = OnceLock::new();
    TYPED.get_or_init(|| transform_with_marker_package(copy_src()))
}

fn copy_typed_full_marker_eval() -> &'static typecheck::TypedFileAst {
    static TYPED: OnceLock<typecheck::TypedFileAst> = OnceLock::new();
    TYPED.get_or_init(|| support::transform_with_full_marker_eval(copy_src()))
}

#[test]
fn hover_is_copy_shows_source_signature() {
    let src = copy_src();
    let typed = copy_typed_marker_package();
    let bind = typed
        .defs
        .values()
        .find(|b| b.name.as_str() == "is_copy")
        .expect("is_copy");
    let span = typed.span_table.get(bind.name_span);
    let (line, character) = src.byte_offset_to_position(span.start());
    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on is_copy");
    assert_eq!(
        hover, "```gin\nis_copy(x Type) Bool\n```",
        "hover on is_copy"
    );
}

#[test]
fn hover_primitive_variant_in_is_copy_when() {
    let src = copy_src();
    let typed = copy_typed_marker_package();
    let prim = src.find("Primitive(_, _)").expect("Primitive arm");
    let (line, character) = src.byte_offset_to_position(prim);
    let hover = typed
        .hover_at(src, line, character)
        .expect("hover on Primitive");
    assert_eq!(
        hover, "```gin\nPrimitive(width in 0...18446744073709551615, signed Bool)\n```",
        "hover on Primitive"
    );
}

#[test]
fn hover_primitive_wildcard_slots() {
    let src = copy_src();
    let typed = copy_typed_marker_package();
    let arm = "    Primitive(_, _)     then True";
    let arm_start = src.find(arm).expect("Primitive arm");
    let underscores: Vec<usize> = arm
        .char_indices()
        .filter(|(_, c)| *c == '_')
        .map(|(i, _)| arm_start + i)
        .collect();
    assert_eq!(underscores.len(), 2, "expected two wildcards in arm");
    let (line, character) = src.byte_offset_to_position(underscores[0]);
    let hover = typed.hover_at(src, line, character).expect("hover first _");
    assert_eq!(
        hover, "```gin\n_: `in 0...18446744073709551615`\n```",
        "hover first _"
    );
    let second_us = underscores[1];
    let (line, character) = src.byte_offset_to_position(second_us);
    let hover = typed
        .hover_at(src, line, character)
        .expect("hover second _");
    assert_eq!(hover, "```gin\n_: `Bool`\n```", "hover second _");
}

#[test]
fn hover_all_variants_copy_param_and_when_subject() {
    let src = copy_src();
    let typed = copy_typed_full_marker_eval();
    let eval_typed = support::marker_eval_typed_for_tests();
    let index = typecheck::PackageSemanticIndex::from_typed_asts(&[eval_typed, typed]);

    let def = "all_variants_copy(variants List(VariantShape)) Bool := when variants is";
    let def_start = src.find(def).expect("all_variants_copy definition");

    let param_pos = def_start + def.find("variants List").expect("variants param");
    let (line, character) = src.byte_offset_to_position(param_pos + 1);
    let param_hover = typed
        .hover_at_with_package(src, line, character, Some(&index))
        .expect("hover variants parameter")
        .markdown;

    let subject_pos = def_start + def.rfind("variants").expect("variants subject");
    let (line, character) = src.byte_offset_to_position(subject_pos + 1);
    let subject_hover = typed
        .hover_at_with_package(src, line, character, Some(&index))
        .expect("hover variants when subject")
        .markdown;

    let expected = "```gin\nvariants List(VariantShape)\n```";
    assert_eq!(param_hover, expected);
    assert_eq!(subject_hover, expected);
}

#[test]
fn hover_fields_in_pattern_and_call_use_same_binding_format() {
    let src = copy_src();
    let typed = copy_typed_full_marker_eval();
    let eval_typed = support::marker_eval_typed_for_tests();
    let index = typecheck::PackageSemanticIndex::from_typed_asts(&[eval_typed, typed]);

    let arm = "    Record(_, fields)   then all_named_copy(fields)";
    let arm_start = src.find(arm).expect("Record arm");
    let pattern_fields = arm_start + arm.find("fields").expect("fields in pattern");
    let (line, character) = src.byte_offset_to_position(pattern_fields);
    let pattern_hover = typed
        .hover_at_with_package(src, line, character, Some(&index))
        .expect("hover fields in pattern")
        .markdown;

    let call_fields = arm_start + arm.rfind("fields").expect("fields in call");
    let (line, character) = src.byte_offset_to_position(call_fields);
    let call_hover = typed
        .hover_at_with_package(src, line, character, Some(&index))
        .expect("hover fields in call")
        .markdown;

    // Pattern site wraps in code fences; call site (cross-file via marker
    // package fixture) uses a thematic break instead.
    assert_eq!(
        pattern_hover, "```gin\nfields List(NamedTy)\n```",
        "pattern binder site"
    );
    assert_eq!(call_hover, "fields List(NamedTy)\n---\n\n", "call site");
}
