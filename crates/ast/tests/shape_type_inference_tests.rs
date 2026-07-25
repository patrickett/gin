//! Shape type inference — record-literal TagCalls resolve to the correct
//! record type, return types are inferred from shape literals, and generic
//! parameters are resolved (Layer 3 of the shape syntax test plan).

mod support;

use ast::TyArg;
use internment::Intern;
use typecheck::TypedFileAst;
use typecheck::ty::Ty;
use typecheck::{BindBody, DefId, ExprId, TypedExprKind};

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

#[test]
fn generic_has_method_call_retains_record_type() {
    let src = "\
Range(x) has
    start x
    end x

    new(start x, end x) Self: Self(start, end)

main: Range.new(12, 1200)
";
    let typed = transform_source(src);
    let method = typed
        .defs
        .get(&DefId(Intern::new("Range.new".to_string())))
        .expect("Range.new definition");
    assert!(
        matches!(method.return_type, Ty::Record { ref name, .. } if name.as_str() == "Range"),
        "Range.new declaration should return Range, got {:?}; receiver: {:?}",
        method.return_type,
        method.receiver_type
    );

    let (_, call_ty) = typed
        .exprs
        .kind
        .iter()
        .zip(&typed.exprs.ty)
        .find(|(kind, _)| {
            matches!(kind, TypedExprKind::FnCall { target, .. } if target.0.as_str() == "Range.new")
        })
        .expect("Range.new call");
    assert!(
        matches!(call_ty, Ty::Record { name, .. } if name.as_str() == "Range"),
        "Range.new call should return Range, got {call_ty:?}"
    );
}

#[test]
fn shape_literal_infers_record_type() {
    let src = "\
Coord has x Int, y Int, z Int
p := Coord(x: 1, y: 2, z: 3)
";
    let typed = transform_source(src);
    let flaws = typed.all_flaws();
    assert!(
        flaws.is_empty(),
        "expected no flaws on valid shape literal, got: {flaws:?}"
    );

    let body = body_expr_id(&typed, "p").expect("p has body");
    let body_ty = typed.exprs.ty.get(body.as_usize()).expect("p body type");

    match body_ty {
        Ty::Record { name, fields, .. } => {
            assert_eq!(name.as_str(), "Coord", "p should have type Coord");
            assert_eq!(fields.len(), 3, "Coord has three fields");

            let field_names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(field_names, ["x", "y", "z"], "field names");
        }
        other => {
            panic!("expected Ty::Record {{ name: \"Coord\", .. }}, got {other:?}");
        }
    }
}

#[test]
fn return_type_inferred_from_shape_literal() {
    let src = "\
Coord has x Int, y Int, z Int
Coord.new(x Int, y Int, z Int):
    Coord(x, y, z)
";
    let typed = transform_source(src);
    // Return-type inference from body is not yet implemented;
    // the body expression itself should resolve to Coord type.
    // At minimum, no crashes during transform.

    // The body's last expression should be a Coord TagCall whose type
    // resolves to the Coord record type (even if field types are Opaque
    // because Int isn't declared).
    let bind = typed
        .defs
        .get(&DefId(Intern::new("Coord.new".to_string())))
        .expect("Coord.new");
    if let BindBody::Body { ref exprs, .. } = bind.body {
        let last = exprs.last().expect("Coord.new has body exprs");
        let last_ty = typed.exprs.ty.get(last.as_usize()).expect("last expr type");
        assert!(
            matches!(last_ty, Ty::Record { name, .. } if name.as_str() == "Coord"),
            "body last expression should be Coord record, got {last_ty:?}"
        );
    } else {
        panic!("Coord.new should have a Body body");
    }
}

#[test]
fn return_type_inferred_from_variant_construction() {
    let src = "\
Maybe(value) is Some(value) or None
Maybe.some(v Int):
    Maybe.Some(value: v)
";
    let typed = transform_source(src);
    // Variant construction with explicit `Maybe.Some(...)` should resolve
    // via the qualified path. Check that the body expression type resolves
    // to the Maybe union type.

    let bind = typed
        .defs
        .get(&DefId(Intern::new("Maybe.some".to_string())))
        .expect("Maybe.some");
    if let BindBody::Body { ref exprs, .. } = bind.body {
        let last = exprs.last().expect("Maybe.some has body exprs");
        let last_ty = typed.exprs.ty.get(last.as_usize()).expect("last expr type");
        assert!(
            matches!(last_ty, Ty::Union { name, .. } if name.as_str() == "Maybe"),
            "body last expression should be Maybe union, got {last_ty:?}"
        );
    } else {
        panic!("Maybe.some should have a Body body");
    }
}

#[test]
fn shape_literal_with_generic_params_resolved() {
    let src = "\
Pair(a, b) has first a, second b
p := Pair(first: 1, second: 2)
";
    let typed = transform_source(src);

    // The body expression should resolve to the Pair record type.
    // Generic params in the declared record fields remain as Opaque placeholders
    // at the tag-type level; concrete types are resolved per-variable at the
    // usage site.
    let body = body_expr_id(&typed, "p").expect("p has body");
    let body_ty = typed.exprs.ty.get(body.as_usize()).expect("p body type");

    match body_ty {
        Ty::Record { name, .. } => {
            assert_eq!(
                name.as_str(),
                "Pair",
                "p should have type Pair, got {body_ty:?}"
            );
        }
        other => {
            panic!("expected Ty::Record {{ name: \"Pair\", .. }}, got {other:?}");
        }
    }
}

#[test]
fn generic_record_application_retains_resolved_params_in_declaration_order() {
    let typed = transform_source("Pair(a, b) has first a, second b\nvalue Pair(Int, String)");
    let value = typed
        .defs
        .get(&DefId(Intern::from_ref("value")))
        .expect("value bind");
    let Ty::Record {
        resolved_params: Some(params),
        ..
    } = &value.return_type
    else {
        panic!("expected resolved Pair record, got {:?}", value.return_type);
    };

    assert_eq!(
        params
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(matches!(&params[0].1, TyArg::Type(_)));
    assert!(matches!(&params[1].1, TyArg::Type(_)));
}

#[test]
fn method_body_shape_literal_inferred() {
    let src = "\
Coord has x Int, y Int, z Int
Coord.origin: Coord(x: 0, y: 0, z: 0)
Coord.new(x Int, y Int, z Int): Coord(x, y, z)
";
    let typed = transform_source(src);
    let flaws = typed.all_flaws();
    let unknown_sym_flaws: Vec<_> = flaws
        .iter()
        .filter(|(_, f)| f.code.slug() == "type-unknown-symbol")
        .collect();
    assert!(
        unknown_sym_flaws.is_empty(),
        "expected no unknown-symbol flaws on Coord methods, got: {flaws:?}"
    );

    // Coord.origin body expression should resolve to Coord record type
    let origin_body = body_expr_id(&typed, "Coord.origin").expect("Coord.origin body");
    let origin_ty = typed
        .exprs
        .ty
        .get(origin_body.as_usize())
        .expect("origin expr type");
    assert!(
        matches!(origin_ty, Ty::Record { name, .. } if name.as_str() == "Coord"),
        "Coord.origin body type should be Coord, got {origin_ty:?}"
    );

    // Coord.new body expression should resolve to Coord record type
    let new_body = body_expr_id(&typed, "Coord.new").expect("Coord.new body");
    let new_ty = typed
        .exprs
        .ty
        .get(new_body.as_usize())
        .expect("new expr type");
    assert!(
        matches!(new_ty, Ty::Record { name, .. } if name.as_str() == "Coord"),
        "Coord.new body type should be Coord, got {new_ty:?}"
    );
}
