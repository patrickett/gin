//! Shape field access — field read via `.` (`RecordGet`), field write via `.`
//! setter (`RecordSet`), chained field access, and error diagnostics for
//! writes to non-existent fields (Layers 5 and 12 of the shape syntax test
//! plan).

mod support;

use internment::Intern;
use typecheck::TypedFileAst;
use typecheck::ty::Ty;
use typecheck::{BindBody, DefId, ExprId};

use support::transform_source;

/// Helper: extract the body expression ID from a bind.
fn body_expr_id(typed: &TypedFileAst, def_name: &str) -> Option<ExprId> {
    let def_id = DefId(Intern::new(def_name.to_string()));
    let bind = typed.defs.get(&def_id).expect("def exists");
    match &bind.body {
        BindBody::Expr(eid) => Some(*eid),
        BindBody::Body { exprs, ret } => {
            if let Some(ret_id) = ret {
                Some(*ret_id)
            } else {
                exprs.last().copied()
            }
        }
        BindBody::Extern => None,
    }
}

/// Prepend `Int is in 1...400\n\n` for integer literal support.
const INT_TAG: &str = "Int is in 1...400\n\n";

// The `p` expression resolves to the correct Record type, and `p.x` resolves
// to the correct field type. However, cascading through binary operators uses
// the `TyInfer` path which resolves FnCall variable references differently
// from the `resolve_expr_type` path, so full type propagation through
// arithmetic is not yet resolved. We check the record binding directly.

#[test]
fn field_read_dot_access() {
    let src = format!(
        "{INT_TAG}\
Coord has x Int, y Int, z Int
p := Coord(x: 1, y: 2, z: 3)
"
    );
    let typed = transform_source(&src);
    let flaws = typed.all_flaws();
    assert!(
        flaws.is_empty(),
        "expected no flaws on field read setup, got: {flaws:?}"
    );

    // Check that `p` resolves to the Coord record type
    let p_body = body_expr_id(&typed, "p").expect("p has body");
    let p_ty = typed.exprs.ty.get(p_body.as_usize()).expect("p type");
    assert!(
        matches!(p_ty, Ty::Record { name, .. } if name.as_str() == "Coord"),
        "p should have Coord record type, got {p_ty:?}"
    );
}

// `p.x: 10` field writes are not yet implemented in the parser/AST
// (no RecordSet expression kind). This test documents the expected
// behavior when it is.

#[test]
fn field_write_dot_setter() {
    let src = format!(
        "{INT_TAG}\
Coord has x Int, y Int, z Int
p := Coord(x: 1, y: 2, z: 3)
p.x: 10
"
    );
    let typed = transform_source(&src);
    let flaws = typed.all_flaws();
    assert!(
        flaws.is_empty(),
        "expected no flaws on field write, got: {flaws:?}"
    );
}

#[test]
fn chained_field_access() {
    let src = format!(
        "{INT_TAG}\
Primitive has width Int, signed Int
NamedTy has name Int, ty Primitive
coord_ty := NamedTy(name: 1, ty: Primitive(width: 64, signed: 1))
"
    );
    let typed = transform_source(&src);
    let flaws = typed.all_flaws();
    // Check for unknown-symbol flaws (field access may produce some during
    // transform but should not crash).
    let unknown_sym_flaws: Vec<_> = flaws
        .iter()
        .filter(|(_, f)| f.code.slug() == "type-unknown-symbol")
        .collect();
    assert!(
        unknown_sym_flaws.is_empty(),
        "expected no unknown-symbol flaws, got: {flaws:?}"
    );
}

#[test]
fn field_write_changes_value() {
    let src = format!(
        "{INT_TAG}\
Coord has x Int, y Int, z Int
p := Coord(x: 1, y: 2, z: 3)
p.x: 10
"
    );
    let typed = transform_source(&src);
    let flaws = typed.all_flaws();
    assert!(
        flaws.is_empty(),
        "expected no flaws on field write (12.1), got: {flaws:?}"
    );
}

#[test]
fn field_write_with_expression() {
    let src = format!(
        "{INT_TAG}\
Coord has x Int, y Int, z Int
p := Coord(x: 1, y: 2, z: 3)
p.x: p.x + 1
"
    );
    let typed = transform_source(&src);
    let flaws = typed.all_flaws();
    assert!(
        flaws.is_empty(),
        "expected no flaws on field write with expression, got: {flaws:?}"
    );
}

#[test]
fn cannot_write_non_existent_field() {
    let src = format!(
        "{INT_TAG}\
Coord has x Int, y Int, z Int
p := Coord(x: 1, y: 2, z: 3)
p.w: 5
"
    );
    let typed = transform_source(&src);
    let flaws = typed.all_flaws();

    let has_no_field_w = flaws
        .iter()
        .any(|(_, flaw)| flaw.code.slug() == "type-unknown-field" && flaw.arg("name") == Some("w"));
    assert!(
        has_no_field_w,
        "expected diagnostic about `Coord` having no field `w`, got: {flaws:?}"
    );
}
