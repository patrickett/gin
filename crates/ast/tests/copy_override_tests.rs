//! Explicit `Copy.can_copy` provisions override the auto `is_copy` default.

mod support;

use ast::ConstValue;
use ast::source::SourceExt;

use internment::Intern;
use support::{marker_trait_registry, transform_with_marker_package};
use typecheck::analysis::TyCopyExt;
use typecheck::trait_field_for_ty;
use typecheck::{BindBody, DefId, TagId};

const INT_TAG: &str = "Int is in 1...400\n\n";

fn source_with_copy_override() -> String {
    format!(
        "{INT_TAG}\
Coord has x Int, y Int

UniqueId has id Int
UniqueId.Copy has can_copy: False
"
    )
}

#[test]
fn copy_auto_default_without_override() {
    let src = format!("{INT_TAG}Coord has x Int, y Int\n");
    let typed = transform_with_marker_package(&src);
    let registry = marker_trait_registry();
    let ty = typed
        .tag_types
        .get(&TagId(Intern::new("Coord".to_string())))
        .expect("Coord");
    assert!(ty.is_copyable(&registry, &typed));
}

#[test]
fn copy_override_false_on_copyable_record() {
    let typed = transform_with_marker_package(&source_with_copy_override());
    let registry = marker_trait_registry();
    let coord = typed
        .tag_types
        .get(&TagId(Intern::new("Coord".to_string())))
        .expect("Coord");
    let unique = typed
        .tag_types
        .get(&TagId(Intern::new("UniqueId".to_string())))
        .expect("UniqueId");
    assert!(coord.is_copyable(&registry, &typed));
    assert!(!unique.is_copyable(&registry, &typed));
}

#[test]
fn copy_override_wins_over_auto_default() {
    let typed = transform_with_marker_package(&source_with_copy_override());
    let registry = marker_trait_registry();
    let ty = typed
        .tag_types
        .get(&TagId(Intern::new("UniqueId".to_string())))
        .expect("UniqueId");
    let cv = trait_field_for_ty("Copy", "can_copy", ty, &typed, &registry).expect("can_copy");
    let ConstValue::Tag { name, .. } = &cv else {
        panic!("expected Bool tag, got {cv:?}");
    };
    assert_eq!(name.as_str(), "False");
}

#[test]
fn hover_shows_not_copy_for_override() {
    let typed = transform_with_marker_package(&source_with_copy_override());
    let src = source_with_copy_override();
    let byte = src.find("UniqueId").expect("UniqueId in source");
    let (line, character) = src.byte_offset_to_position(byte);
    let hover = typed
        .hover_at(&src, line, character)
        .expect("hover on UniqueId");
    assert_eq!(
        hover, "```gin\nUniqueId has id Int\n```",
        "expected hover on UniqueId"
    );
}

#[test]
fn non_copy_override_use_after_move() {
    let src = format!(
        "{INT_TAG}\
UniqueId has id Int
UniqueId.Copy has can_copy: False

drop(eat x Int) Int: 0

main:
    u: UniqueId(42)
    drop(eat u)
    v: u
return 0
"
    );
    let typed = transform_with_marker_package(&src);
    let mut diags = Vec::new();
    for (_, flaw) in typed.all_flaws() {
        diags.push(flaw.clone());
    }
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(
        has_moved,
        "expected UseOfMovedValue for non-Copy UniqueId, got: {diags:?}"
    );
}

#[test]
fn sum_coords_field_access_no_flaws() {
    let src = format!(
        "{INT_TAG}\
Coord has x Int, y Int\n\
\n\
sum_coords(a Coord, b Coord) Int: a.x + b.x + a.y + b.y\n"
    );
    let typed = transform_with_marker_package(&src);
    let flaws = typed.all_flaws();
    assert!(flaws.is_empty(), "expected no flaws, got: {flaws:?}");

    let sum_coords = typed
        .defs
        .get(&DefId(Intern::new("sum_coords".to_string())))
        .expect("sum_coords");
    let ret = match &sum_coords.body {
        BindBody::Expr(id) => *id,
        BindBody::Body { ret: Some(id), .. } => *id,
        other => panic!("expected expression body, got {other:?}"),
    };
    let ret_ty = typed.exprs.ty.get(ret.as_usize()).expect("return ty");
    assert!(
        ret_ty.is_int(),
        "sum of Int fields should be Int, got {ret_ty:?}"
    );
}
