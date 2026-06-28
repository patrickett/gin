//! `modules/gin_core/marker/copy.gin` documents opt-out via `Type.Copy(can_copy: False)`.

mod support;

const COPY_GIN: &str = r#"use core.reflect.(Type, NamedTy, VariantShape)
use core.primitive.(Bool, List)

--- Types that can be implicitly copied.
--- Opt out with `Type.Copy(can_copy: False)`.
Copy has (can_copy Bool)

-- Blanket: copyability follows structural rules on reflected shape.
x.Copy(can_copy: is_copy(x))

is_copy(x Type) Bool := when x is
    Primitive(_, _)     then True
    Ptr(_)              then False
    Ref(_, False)       then True
    Ref(_, True)        then False
    Opaque(_)           then False
    Record(_, fields)   then all_named_copy(fields)
    Tuple(elems)        then all_types_copy(elems)
    Union(_, variants)  then all_variants_copy(variants)
    Array(elem, _)      then is_copy(elem)

all_named_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_types_copy(elems List(Type)) Bool := when elems is
    []                  then True
    [t, ...rest]        then is_copy(t) and all_types_copy(rest)

all_variants_copy(variants List(VariantShape)) Bool := when variants is
    []                  then True
    [v, ...rest]        then all_named_copy(v.fields) and all_variants_copy(rest)
"#;

#[test]
fn copy_gin_documents_trait_override_opt_out() {
    let source = COPY_GIN;
    let _opt_out_pos = source
        .find("Type.Copy(can_copy: False)")
        .expect("copy.gin should document trait override opt-out");
    let file_ast = parser::cursor::TokenCursor::parse_source(source);
    let typed = typecheck::transform::transform_declare(
        &file_ast,
        typecheck::FileId(0),
        support::marker_transform_ctx().as_ref(),
    );
    assert!(
        typed.declaration_flaws.is_empty(),
        "copy.gin should transform cleanly: {:?}",
        typed.declaration_flaws
    );
}
