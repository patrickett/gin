//! Integration tests for dependent type refinement checks.
//!
//! Validates that the flow-time refinement checker (`check_tag_refinements` in
//! `transform/flow.rs`) correctly catches violations and respects flow facts.

use internment::Intern;
use typecheck::ty::Ty;
use typecheck::{TypedExprKind, TypedFileAst};

mod support;
use support::transform_source_with_typed_locals;
use support::{body_expr_id, has_flaw_with_slug as has_flaw};

fn flaw_count(typed: &TypedFileAst, slug: &str) -> usize {
    typed
        .all_flaws()
        .iter()
        .filter(|(_, f)| f.code.slug() == slug)
        .count()
}

#[test]
fn refinement_accepted_with_literal() {
    let typed = transform_source_with_typed_locals(
        "Index has \n\
             n Int,\n\
             value Int and < n,\n\
         )\n\
         i := Index(3, 2)\n",
    );
    assert!(
        !has_flaw(&typed, "dep-refinement-failed"),
        "expected no dep-refinement-failed, got: {:?}",
        typed.all_flaws()
    );
    assert!(
        !has_flaw(&typed, "dep-refinement-unproven"),
        "expected no dep-refinement-unproven, got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn refinement_accepted_with_negative_literal() {
    let typed = transform_source_with_typed_locals(
        "Index has \n\
             n Int,\n\
             value Int and < n,\n\
         )\n\
         i := Index(3, -1)\n",
    );
    assert!(
        !has_flaw(&typed, "dep-refinement-failed"),
        "expected no dep-refinement-failed for Index(3, -1), got: {:?}",
        typed.all_flaws()
    );
    assert!(
        !has_flaw(&typed, "dep-refinement-unproven"),
        "expected no dep-refinement-unproven for Index(3, -1), got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn refinement_rejected_with_literal() {
    let typed = transform_source_with_typed_locals(
        "Index has \n\
             n Int,\n\
             value Int and < n,\n\
         )\n\
         i := Index(3, 5)\n",
    );
    assert!(
        has_flaw(&typed, "dep-refinement-failed"),
        "expected dep-refinement-failed for Index(3, 5), got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn refinement_unknown_only_in_fn_body() {
    let typed = transform_source_with_typed_locals(
        "Index has \n\
             n Int,\n\
             value Int and < n,\n\
         )\n\
         i := Index(3, 2)\n\
         test(n Int, value Int):\n\
             Index(n, value)\n",
    );
    let failed_count = flaw_count(&typed, "dep-refinement-failed");
    let unproven_count = flaw_count(&typed, "dep-refinement-unproven");
    // i := Index(3, 2) should pass (no flaw)
    // test body Index(n, value) should get unproven
    assert_eq!(
        failed_count,
        0,
        "expected 0 dep-refinement-failed, got {}: {:?}",
        failed_count,
        typed.all_flaws()
    );
    assert_eq!(
        unproven_count,
        1,
        "expected 1 dep-refinement-unproven, got {}: {:?}",
        unproven_count,
        typed.all_flaws()
    );
}

#[test]
fn no_refinement_no_error() {
    let typed = transform_source_with_typed_locals(
        "Pair has \n\
             x Int,\n\
             y Int,\n\
         )\n\
         p := Pair(3, 5)\n",
    );
    assert!(
        !has_flaw(&typed, "dep-refinement-failed"),
        "expected no dep-refinement-failed for plain record, got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn refinement_multiple_fields() {
    let typed = transform_source_with_typed_locals(
        "Range has \n\
             low Int,\n\
             high Int and < low,\n\
         )\n\
         r := Range(5, 1)\n",
    );
    assert!(
        !has_flaw(&typed, "dep-refinement-failed"),
        "expected Range(1, 5) to be accepted, got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn refinement_multiple_fields_rejected() {
    let typed = transform_source_with_typed_locals(
        "Range has \n\
             low Int,\n\
             high Int and < low,\n\
         )\n\
         r := Range(1, 5)\n",
    );
    assert!(
        has_flaw(&typed, "dep-refinement-failed"),
        "expected Range(5, 2) to be rejected, got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn named_args_refinement() {
    let typed = transform_source_with_typed_locals(
        "Index has \n\
             n Int,\n\
             value Int and < n,\n\
         )\n\
         i := Index(value: 2, n: 3)\n",
    );
    assert!(
        !has_flaw(&typed, "dep-refinement-failed"),
        "expected named-arg Index(value: 2, n: 3) to be accepted, got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn named_args_refinement_rejected() {
    let typed = transform_source_with_typed_locals(
        "Index has \n\
             n Int,\n\
             value Int and < n,\n\
         )\n\
         i := Index(value: 5, n: 3)\n",
    );
    assert!(
        has_flaw(&typed, "dep-refinement-failed"),
        "expected named-arg Index(value: 5, n: 3) to be rejected, got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn refinement_accepted_in_if_body() {
    // Plan 16: inside `if value < n:`, the flow analysis should know
    // that value < n and accept Index(n, value).
    // The `<` desugars to `lt(a, b)` which must be a known function.
    let typed = transform_source_with_typed_locals(
        "lt(a Int, b Int) Int := return 0\n         Index has \n             n Int,\n             value Int and < n,\n         )\n         test(n Int, value Int):\n             when value < n: then\n                 Index(n, value)\n",
    );
    assert!(
        !has_flaw(&typed, "dep-refinement-failed"),
        "expected Index(n, value) inside if-body to be accepted, got: {:?}",
        typed.all_flaws()
    );
    assert!(
        !has_flaw(&typed, "dep-refinement-unproven"),
        "expected Index(n, value) inside if-body to be accepted, got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn refinement_rejected_outside_if_body() {
    let typed = support::transform_source(
        "Int is in 0 to 400\nInt has\n    as_num(self ref) is in Self: return (number is in Self: self)\n    finish(self): return\nIndex has\n    n Int\n    value Int and < n\n    finish(self): return\ntest(n Int, value Int) Index:\n    when value.as_num < n.as_num then\n        checked: Index(n, value)\n        checked.finish\n    return Index(n, value)\n",
    );
    assert_eq!(
        flaw_count(&typed, "dep-refinement-unproven"),
        1,
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn indexed_adt_parses() {
    let typed = transform_source_with_typed_locals(
        "Vec(n Int) is\n\
             Nil -> Vec(0)\n\
             or Cons(head Int, tail Vec(n)) -> Vec(n)\n",
    );
    let vec_id = typecheck::TagId(Intern::new("Vec".to_string()));
    let tag = typed.tags.get(&vec_id).expect("Vec tag exists");
    if let Ty::Union { variants, .. } = typed
        .type_registry
        .resolved_definition_for_type(&tag.resolved_ty)
    {
        assert_eq!(variants.len(), 2, "two variants");
        let nil = &variants[0];
        assert_eq!(nil.name.as_str(), "Nil");
        assert!(nil.result_ty.is_some(), "Nil should have result_ty");
        let cons = &variants[1];
        assert_eq!(cons.name.as_str(), "Cons");
        assert!(cons.result_ty.is_some(), "Cons should have result_ty");
    } else {
        panic!("Expected Union, got {:?}", tag.resolved_ty);
    }
}

#[test]
fn const_generic_union_stores_resolved_params() {
    // When a union type is applied with concrete params (e.g. `Vec(3)`),
    // the resolved Ty should carry those params in `resolved_params`.
    let typed = transform_source_with_typed_locals(
        "Vec(n Int) is\n\
             Nil -> Vec(0)\n\
             or Cons(head Int, tail Vec(n)) -> Vec(n)\n\
         v Vec(3)\n",
    );
    let vec_ty = typed
        .tag_types
        .get(&typecheck::TagId(Intern::new("Vec".to_string())))
        .cloned()
        .expect("Vec type");
    // The base union type should NOT have resolved_params (none set on the tag itself).
    if let Ty::Union {
        resolved_params, ..
    } = typed.type_registry.resolved_definition_for_type(&vec_ty)
    {
        assert!(
            resolved_params.is_none(),
            "base union type should not have resolved_params; found: {:?}",
            resolved_params
        );
    } else {
        panic!("Expected Union, got {:?}", vec_ty);
    }
}

#[test]
fn pattern_bindings_substituted_from_resolved_params() {
    for (argument, count) in [("3", 3), ("7", 7), ("n: 3", 3), ("n: 7", 7)] {
        let source = format!(
            "Int is in 0 to 400\nInt.finish(self): return\nLink(element) has\n    pointer @element\n    finish(self): return\nVec(n Int) is\n    Nil -> Vec(0)\n    or Cons(head Int, tail Link(Vec(n))) -> Vec(n)\nVec(n) has\n    finish(self):\n        when self is\n            Cons(head, tail) then\n                head.finish\n                tail.finish\n            Nil then return\n        return\ntest(v ref Vec({argument})):\n    when v is\n        Cons(_, t) then\n            observed: t\n        Nil then return\n    return\n",
        );
        let parsed = parser::cursor::TokenCursor::parse_source(&source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{argument}: {:?}",
            parsed.parse_warnings
        );
        let typed = typecheck::transform::transform(
            &parsed,
            typecheck::FileId(0),
            &typecheck::transform::TransformCtx::new(),
        );
        assert!(
            typed.all_flaws().is_empty(),
            "{argument}: {:?}",
            typed.all_flaws()
        );
        let subject = typed
            .places
            .iter()
            .find(|place| place.name.as_str() == "v")
            .expect("subject parameter");
        let observed = typed
            .places
            .iter()
            .find(|place| place.name.as_str() == "observed")
            .expect("observed tail binding");
        let expected = subject.ty.without_reference_wrappers();
        let Ty::Named { instance, .. } = expected else {
            panic!("expected nominal Vec type, got {expected:?}");
        };
        assert_eq!(
            instance.arguments,
            vec![(Intern::from_ref("n"), ast::TyArg::Const(count.into()))],
            "{argument}"
        );
        let Ty::Ref {
            inner,
            mutable: false,
        } = &observed.ty
        else {
            panic!("expected borrowed tail, got {:?}", observed.ty);
        };
        let Ty::Record { fields, .. } = typed.type_registry.resolved_definition_for_type(inner)
        else {
            panic!("expected Link record, got {inner:?}");
        };
        assert_eq!(
            fields,
            vec![(
                Intern::from_ref("pointer"),
                Box::new(Ty::Ptr {
                    inner: Box::new(expected.clone())
                })
            )],
            "{argument}"
        );
    }
}

#[test]
fn const_generic_call_substituted_return_type() {
    let typed = support::transform_source(
        "Int is in 0 to 400\nInt.finish(self): return\nid_ty(x) Int: return #size(x)\nmain: id_ty(Int)\n",
    );
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");
    // The expression should be an FnCall with 0 runtime args (Int is a type arg).
    // The return type should be Bool (substituted from the declared return type).
    match &expr.kind {
        TypedExprKind::FnCall {
            args,
            substituted_ty,
            ..
        } => {
            assert_eq!(
                args.as_ref().map(Vec::len),
                Some(0),
                "type arg should not be lowered as runtime arg"
            );
            assert!(
                substituted_ty.is_some(),
                "const-generic call should produce substituted_ty"
            );
        }
        other => panic!("Expected FnCall, got {:?}", other),
    }
}

#[test]
fn const_generic_call_substituted_ty_is_record() {
    let typed = transform_source_with_typed_locals(
        "Pair has x Int, y Int\n\
         make_pair(x) Pair := Pair(0, 0)\n\
         main: make_pair(Int)\n",
    );
    let main_body = body_expr_id(&typed, "main").expect("main has body");
    let expr = typed.exprs.get(main_body.as_usize()).expect("main body");
    match &expr.kind {
        TypedExprKind::FnCall {
            substituted_ty: Some(ty),
            ..
        } => {
            assert!(
                matches!(
                    typed.type_registry.resolved_definition_for_type(ty),
                    Ty::Record { name, .. } if name.as_str() == "Pair"
                ),
                "expected substituted_ty to be Pair record, got {:?}",
                ty
            );
        }
        other => panic!("Expected FnCall with substituted_ty, got {:?}", other),
    }
}
