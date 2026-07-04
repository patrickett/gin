//! Integration tests for dependent type refinement checks.
//!
//! Validates that the flow-time refinement checker (`check_tag_refinements` in
//! `transform/flow.rs`) correctly catches violations and respects flow facts.

use internment::Intern;
use typecheck::ty::Ty;
use typecheck::{BindBody, DefId, ExprId, TypedExprKind, TypedFileAst};

mod support;
use support::transform_source_with_typed_locals;

fn has_flaw(typed: &TypedFileAst, slug: &str) -> bool {
    typed.all_flaws().iter().any(|(_, f)| f.code.slug() == slug)
}

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
        "Index has (\n\
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
fn refinement_rejected_with_literal() {
    let typed = transform_source_with_typed_locals(
        "Index has (\n\
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
fn refinement_unknown_for_variable_arg() {
    let typed = transform_source_with_typed_locals(
        "Index has (\n\
             n Int,\n\
             value Int and < n,\n\
         )\n\
         test(n Int, value Int):\n\
             Index(n, value)\n",
    );
    assert!(
        has_flaw(&typed, "dep-refinement-unproven"),
        "expected dep-refinement-unproven for variable args, got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn refinement_unknown_only_in_fn_body() {
    let typed = transform_source_with_typed_locals(
        "Index has (\n\
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
        "Pair has (\n\
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
        "Range has (\n\
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
        "Range has (\n\
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
        "Index has (\n\
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
        "Index has (\n\
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
        "lt(a Int, b Int) Int := 0\n\
         Index has (\n\
             n Int,\n\
             value Int and < n,\n\
         )\n\
         test(n Int, value Int):\n\
             if value < n:\n\
                 Index(n, value)\n",
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
    // Outside the if-guard, the flow analysis shouldn't know value < n.
    let typed = transform_source_with_typed_locals(
        "lt(a Int, b Int) Int := 0\n\
         Index has (\n\
             n Int,\n\
             value Int and < n,\n\
         )\n\
         test(n Int, value Int):\n\
             if_check := if value < n:\n\
                 0\n\
             return 0\n\
\n\
             outside_if := Index(n, value)\n\
\n\
         return 0\n",
    );
    assert!(
        has_flaw(&typed, "dep-refinement-unproven"),
        "expected Index(n, value) outside if-body to be unproven, got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn const_generic_call_substitutes_return_type() {
    let typed = transform_source_with_typed_locals(
        "id_ty(x Type) Bool := Bool.True\n\
         main: id_ty(Int)\n",
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
        "Pair has (x Int, y Int)\n\
         make_pair(x Type) Pair := Pair(0, 0)\n\
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
                matches!(ty, Ty::Record { name, .. } if name.as_str() == "Pair"),
                "expected substituted_ty to be Pair record, got {:?}",
                ty
            );
        }
        other => panic!("Expected FnCall with substituted_ty, got {:?}", other),
    }
}

/// Helper: extract a bind body ExprId from a TypedBind.
fn body_expr_id(typed: &TypedFileAst, def_name: &str) -> Option<ExprId> {
    let def_id = DefId(Intern::new(def_name.to_string()));
    let bind = typed.defs.get(&def_id)?;
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
