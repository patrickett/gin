//! Backward-compatibility tests for shape, trait, and method syntax (Layer 11).
//!
//! These tests document that existing `.gin` source patterns continue to
//! parse and transform without regressions.

use ast::ty::Ty;
use internment::Intern;
use typecheck::{DefId, TagId};

mod support;
use support::transform_source;

/// 11.1 — Trait declarations (trait shape with trait fields).
#[test]
fn trait_declarations() {
    let src = "\
Copy has (can_copy Bool)
Sized has (size Int)
Default(value) has (default value)";
    let typed = transform_source(src);

    // All three should be valid record tags.
    for name in &["Copy", "Sized", "Default"] {
        let id = TagId(Intern::new(name.to_string()));
        let tag = typed
            .tags
            .get(&id)
            .unwrap_or_else(|| panic!("{name} tag should exist"));
        assert!(
            matches!(&tag.resolved_ty, Ty::Record { .. }),
            "{name} should be a Record, got {:?}",
            tag.resolved_ty
        );
    }

    // `Default` should have a generic parameter `value`.
    let default_id = TagId(Intern::new("Default".to_string()));
    let def_tag = typed.tags.get(&default_id).expect("Default");
    assert!(
        def_tag.params.is_some(),
        "Default should have generic params"
    );
    if let Some(params) = &def_tag.params {
        assert!(
            params.contains_key(&Intern::new("value".to_string())),
            "Default should have 'value' param"
        );
    }

    let flaws = typed.all_flaws();
    let unknown: Vec<_> = flaws
        .iter()
        .filter(|(_, f)| f.code.slug() == "type-unknown-symbol")
        .collect();
    assert!(unknown.is_empty(), "no unknown-symbol flaws: {unknown:?}");
}

/// 11.2 — Interface method declarations (shape with return and error types).
#[test]
fn interface_method_declarations() {
    let src = "\
Slice(Byte) has (pointer Int, length Int)
AllocError is Oom or InvalidLayout
Allocator has (
    reserve(ref self, l Int) Slice(Byte) or AllocError,
    release(ref self, p Int, l Int),
)";
    let typed = transform_source(src);

    for name in &["Slice", "AllocError", "Allocator"] {
        let id = TagId(Intern::new(name.to_string()));
        assert!(typed.tags.contains_key(&id), "{name} tag should exist");
    }

    // Allocator should be a Record (interface shape).
    let allocator_id = TagId(Intern::new("Allocator".to_string()));
    let allocator = typed.tags.get(&allocator_id).expect("Allocator");
    assert!(
        matches!(&allocator.resolved_ty, Ty::Record { .. }),
        "Allocator should be a Record, got {:?}",
        allocator.resolved_ty
    );

    let flaws = typed.all_flaws();
    let _ = flaws;
}

/// 11.3 — Record shape with constructor (namespaced constructor method).
#[test]
fn record_with_constructor() {
    let src = "\
Int is in 1...400
String has (bytes List(Byte))
Coord has (x Int, y Int)
Coord.make(x Int, y Int) Coord: Coord(x, y)";
    let typed = transform_source(src);

    // Tags should exist.
    for name in &["String", "Coord"] {
        let id = TagId(Intern::new(name.to_string()));
        assert!(typed.tags.contains_key(&id), "{name} tag should exist");
    }

    // `Coord.make` should be a valid function def.
    let make_id = DefId(Intern::new("Coord.make".to_string()));
    assert!(typed.defs.contains_key(&make_id), "Coord.make should exist");

    let flaws = typed.all_flaws();
    let _ = flaws;
}

/// 11.4 — Blanket impl (`x has Trait(...)` with lowercase quantified var).
///
/// Note: `transform_source` uses a bare `TransformCtx` with no pre-loaded
/// blanket impls.  The blanket impl IS recorded on `FileAst` but does not
/// flow into `TypedFileAst.blanket_impls` without a properly populated ctx.
#[test]
fn blanket_impl() {
    let src = "x has Sized(size: 8)";
    let typed = transform_source(src);

    // Should NOT create a tag for `x`.
    assert!(
        !typed
            .tags
            .contains_key(&TagId(Intern::new("x".to_string()))),
        "lowercase blanket subject should not create a tag"
    );

    // Transform should not crash.
    let flaws = typed.all_flaws();
    let _ = flaws;
}

/// 11.5 — Core Bool/Maybe style declarations with `and has Trait(...)`.
#[test]
fn bool_style_and_has_trait() {
    let src = "\
Bool is True or False and has Copy(can_copy: True)";
    let typed = transform_source(src);

    let bool_id = TagId(Intern::new("Bool".to_string()));
    let tag = typed.tags.get(&bool_id).expect("Bool tag exists");

    // The union should have two variants.
    match &tag.resolved_ty {
        Ty::Union { name, variants, .. } => {
            assert_eq!(name.as_str(), "Bool", "union name");
            assert_eq!(variants.len(), 2, "two variants");
        }
        other => panic!("Expected Union, got {other:?}"),
    }

    // The `and has Copy(...)` should produce a ProvidedTrait.
    let has_copy = tag
        .provided_traits
        .iter()
        .any(|pt| pt.trait_name.as_str() == "Copy");
    assert!(
        has_copy,
        "Bool should have a Copy provided trait, got: {:?}",
        tag.provided_traits
    );

    let flaws = typed.all_flaws();
    let _ = flaws;
}
