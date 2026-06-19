//! `when subject is` pattern binders must be in scope in arm bodies.

use std::sync::OnceLock;

mod support;
use support::transform_with_marker_package;

fn copy_src() -> &'static str {
    // Types resolve from the marker package fixture; only need the is_copy
    // definitions whose binder names are being tested.
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
    Tuple(_, elems)     then all_types_copy(elems)
    Union(_, variants)  then all_variants_copy(variants)

all_named_copy(fields) Bool := when fields is
    []                  then True
    [f, ...rest]        then is_copy(f.ty) and all_named_copy(rest)

all_types_copy(elems) Bool := when elems is
    []                  then True
    [t, ...rest]        then is_copy(t) and all_types_copy(rest)

all_variants_copy(variants) Bool := when variants is
    []                  then True
    [v, ...rest]        then all_named_copy(v.fields) and all_variants_copy(rest)
"
}

fn copy_typed() -> &'static typecheck::TypedFileAst {
    static TYPED: OnceLock<typecheck::TypedFileAst> = OnceLock::new();
    TYPED.get_or_init(|| transform_with_marker_package(copy_src()))
}

#[test]
fn is_copy_record_fields_not_unknown_symbol() {
    let typed = copy_typed();

    let unknown_fields: Vec<_> = typed
        .all_flaws()
        .into_iter()
        .filter_map(|(_, flaw)| {
            (flaw.code.slug() == "type-unknown-symbol" && flaw.arg("name") == Some("fields"))
                .then_some("fields")
        })
        .collect();
    assert!(
        unknown_fields.is_empty(),
        "expected `fields` in Record arm to resolve, got flaws: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn is_copy_tuple_elems_not_unknown_symbol() {
    let typed = copy_typed();

    let unknown: Vec<_> = typed
        .all_flaws()
        .into_iter()
        .filter(|&(_, flaw)| {
            flaw.code.slug() == "type-unknown-symbol" && flaw.arg("name") == Some("elems")
        })
        .map(|(_, _flaw)| "elems")
        .collect();
    assert!(
        unknown.is_empty(),
        "expected `elems` in Tuple arm to resolve, got flaws: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn list_cons_head_variable_f_is_not_unknown_symbol() {
    // The `f` in `[f, ...rest]` should be introduced into scope.
    // Use a simpler file with just a when-expression and a direct reference
    // to the bound variable, avoiding recursive calls.
    let source = r#"
use core.reflect/.(NamedTy, Type)
use core.primitive.(List, Bool)

all_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then True
"#;
    let typed = transform_with_marker_package(source);
    let flaws = typed.all_flaws();
    let unknown_f: Vec<_> = flaws
        .iter()
        .filter(|&(_, flaw)| {
            flaw.code.slug() == "type-unknown-symbol" && flaw.arg("name") == Some("f")
        })
        .map(|(_, _flaw)| "f".to_string())
        .collect();
    assert!(
        unknown_f.is_empty(),
        "expected `f` in list cons pattern to resolve, got flaws: {:?}",
        flaws
    );
}

#[test]
fn list_cons_rest_variable_is_not_unknown_symbol() {
    // The `rest` in `[f, ...rest]` should be introduced into scope.
    //
    // Use a simpler file with just a when-expression and a direct reference
    // to the bound variable, avoiding recursive calls.
    let source = r#"
use core.reflect/.(NamedTy, Type)
use core.primitive.(List, Bool)

all_copy(fields List(NamedTy)) Bool := when fields is
    []                  then True
    [f, ...rest]        then True
"#;
    let typed = transform_with_marker_package(source);
    let flaws = typed.all_flaws();
    let unknown_rest: Vec<_> = flaws
        .iter()
        .filter(|&(_, flaw)| {
            flaw.code.slug() == "type-unknown-symbol" && flaw.arg("name") == Some("rest")
        })
        .map(|(_, _flaw)| "rest".to_string())
        .collect();
    assert!(
        unknown_rest.is_empty(),
        "expected `rest` in list cons pattern to resolve, got flaws: {:?}",
        flaws
    );
}

#[test]
fn list_cons_head_variable_t_is_not_unknown_symbol() {
    // The `t` in `[t, ...rest]` (all_types_copy) should be in scope.
    let typed = copy_typed();

    let unknown_t: Vec<_> = typed
        .all_flaws()
        .into_iter()
        .filter_map(|(_, flaw)| {
            (flaw.code.slug() == "type-unknown-symbol" && flaw.arg("name") == Some("t"))
                .then_some("t")
        })
        .collect();
    assert!(
        unknown_t.is_empty(),
        "expected `t` in list cons pattern to resolve, got flaws: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn list_cons_head_variable_v_is_not_unknown_symbol() {
    // The `v` in `[v, ...rest]` (all_variants_copy) should be in scope.
    let typed = copy_typed();

    let unknown_v: Vec<_> = typed
        .all_flaws()
        .into_iter()
        .filter_map(|(_, flaw)| {
            (flaw.code.slug() == "type-unknown-symbol" && flaw.arg("name") == Some("v"))
                .then_some("v")
        })
        .collect();
    assert!(
        unknown_v.is_empty(),
        "expected `v` in list cons pattern to resolve, got flaws: {:?}",
        typed.all_flaws()
    );
}
