//! Integration tests for dependent type refinement checks.
//!
//! Validates that the flow-time refinement checker (`check_tag_refinements` in
//! `transform/flow.rs`) correctly catches violations and respects flow facts.

use typecheck::TypedFileAst;

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
