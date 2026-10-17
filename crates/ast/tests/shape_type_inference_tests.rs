//! Shape type inference — record-literal TagCalls resolve to the correct
//! record type, return types are inferred from shape literals, and generic
//! parameters are resolved (Layer 3 of the shape syntax test plan).

mod support;

use ast::TyArg;
use internment::Intern;
use typecheck::ty::Ty;
use typecheck::{DefId, TypedExprKind};

use support::{body_expr_id, transform_source};

#[test]
fn generic_has_method_call_retains_record_type() {
    let src = "Range(x) has\n    start x\n    end x\n\n    new(start x, end x) Self: return Self(start, end)\n\nmain: Range.new(12, 1200)\n";
    let typed = transform_source(src);
    let method = typed
        .defs
        .get(&DefId::named(Intern::new("Range.new".to_string())))
        .expect("Range.new definition");
    assert!(
        matches!(
            typed.type_registry.resolved_definition_for_type(&method.return_type),
            Ty::Record { name, .. } if name.as_str() == "Range"
        ),
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
            matches!(kind, TypedExprKind::FnCall { target, .. } if target.0.as_str().starts_with("Range.new"))
        })
        .expect("Range.new call");
    assert!(
        matches!(
            typed.type_registry.resolved_definition_for_type(call_ty),
            Ty::Record { name, .. } if name.as_str() == "Range"
        ),
        "Range.new call should return Range, got {call_ty:?}"
    );
}

#[test]
fn shape_literal_infers_record_type() {
    let src = "Int is in 0 to 255\nInt has\n    finish(self):\n        return\nCoord has\n    x Int, y Int, z Int\n    finish(self):\n        return\np := Coord(x: 1, y: 2, z: 3)\n";
    let typed = transform_source(src);
    let flaws = typed.all_flaws();
    assert!(
        flaws.is_empty(),
        "expected no flaws on valid shape literal, got: {flaws:?}"
    );

    let body = body_expr_id(&typed, "p").expect("p has body");
    let body_ty = typed.exprs.ty.get(body.as_usize()).expect("p body type");

    match typed.type_registry.resolved_definition_for_type(body_ty) {
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
    let src = "Int is in 0 to 255\nInt has\n    finish(self):\n        return\nCoord has\n    x Int, y Int, z Int\n    finish(self):\n        return\nCoord.new(x Int, y Int, z Int):\n    return Coord(x, y, z)\n";
    let typed = transform_source(src);
    let body = body_expr_id(&typed, "Coord.new").expect("Coord.new body");
    let ty = &typed.exprs.ty[body.as_usize()];
    assert!(
        matches!(typed.type_registry.resolved_definition_for_type(ty),
        Ty::Record { name, .. } if name.as_str() == "Coord"),
        "{ty:?}"
    );
}

#[test]
fn return_type_inferred_from_variant_construction() {
    let src = "\
Word is in 0 to 255
Word.finish(self): return
Maybe is Some(value Word) or None
Maybe.finish(self): return
Maybe.some(v Word):
    return Maybe.Some(value: v)
";
    let typed = transform_source(src);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let body = body_expr_id(&typed, "Maybe.some").expect("Maybe.some body");
    let ty = &typed.exprs.ty[body.as_usize()];
    assert!(
        matches!(typed.type_registry.resolved_definition_for_type(ty),
        Ty::Union { name, .. } if name.as_str() == "Maybe"),
        "{ty:?}"
    );
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

    match typed.type_registry.resolved_definition_for_type(body_ty) {
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
        .get(&DefId::named(Intern::from_ref("value")))
        .expect("value bind");
    let Ty::Record {
        resolved_params: Some(params),
        ..
    } = typed
        .type_registry
        .resolved_definition_for_type(&value.return_type)
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
    let src = "Int is in 0 to 255\nInt has\n    finish(self):\n        return\nCoord has\n    x Int, y Int, z Int\n    finish(self):\n        return\nCoord.origin: return Coord(x: 0, y: 0, z: 0)\nCoord.new(x Int, y Int, z Int): return Coord(x, y, z)\n";
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
        matches!(
            typed.type_registry.resolved_definition_for_type(origin_ty),
            Ty::Record { name, .. } if name.as_str() == "Coord"
        ),
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
        matches!(
            typed.type_registry.resolved_definition_for_type(new_ty),
            Ty::Record { name, .. } if name.as_str() == "Coord"
        ),
        "Coord.new body type should be Coord, got {new_ty:?}"
    );
}
