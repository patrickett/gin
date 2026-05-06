//! Typecheck tests for generic methods on parameterized types.
//!
//! Covers:
//! - Positive: `Range.new(12, 1200)` typechecks (both args satisfy `x = Int`).
//! - Negative: `Range.new(1, "hi")` rejects (`x` cannot be both `Int` and `Str`).
//! - Contrast: `CustomRange has (start, end)` (no type variable) accepts
//!   `CustomRange.new(1, "hi")` because `start` and `end` are independently
//!   generic.
//! - Tuple-IS-record: the body `(start, end)` satisfies the `Range[x]` record
//!   return without an explicit record literal.

use diagnostic::{DiagnosticCode, TypeSymptom};
use parser::parse_source_full;
use typeck::TyEnv;

const RANGE_DECL: &str =
    "Range[x] has (start x, end x)\n\nRange[x].new(start x, end x) Range[x]: (start, end)\n\n";
const CUSTOM_DECL: &str =
    "CustomRange has (start, end)\n\nCustomRange.new(start, end) CustomRange: (start, end)\n\n";

fn type_mismatch_count(symptoms: &[diagnostic::Diagnostic]) -> usize {
    symptoms
        .iter()
        .filter(|d| matches!(d.code, DiagnosticCode::Type(TypeSymptom::Mismatch)))
        .count()
}

#[test]
fn range_new_with_matching_int_args_typechecks() {
    // Positive: both args are Int, so `x` unifies as Int.
    let src = format!("{RANGE_DECL}main:\n    r := Range.new(12, 1200)\nreturn\n");
    let out = parse_source_full(&src);
    let env = TyEnv::from_file_ast(&out.ast);
    let mut symptoms = Vec::new();
    env.check_unknowns(&out.ast, &mut symptoms);

    assert_eq!(
        type_mismatch_count(&symptoms),
        0,
        "Range.new(12, 1200) should typecheck cleanly, got: {:#?}",
        symptoms
    );
}

#[test]
fn range_new_with_mismatched_arg_types_rejects() {
    // Negative: `x` cannot be both Int and Str — second arg should fail to unify.
    let src = format!("{RANGE_DECL}main:\n    r := Range.new(1, 'hi')\nreturn\n");
    let out = parse_source_full(&src);
    let env = TyEnv::from_file_ast(&out.ast);
    let mut symptoms = Vec::new();
    env.check_unknowns(&out.ast, &mut symptoms);

    assert!(
        type_mismatch_count(&symptoms) >= 1,
        "Range.new(1, 'hi') should produce a type-mismatch diagnostic; got: {:#?}",
        symptoms
    );
}

#[test]
fn custom_range_without_type_var_accepts_mixed_arg_types() {
    // Contrast: `CustomRange has (start, end)` (no `(x)`) leaves both fields
    // independently generic, so the two args don't have to share a type.
    let src = format!("{CUSTOM_DECL}main:\n    r := CustomRange.new(1, 'hi')\nreturn\n");
    let out = parse_source_full(&src);
    let env = TyEnv::from_file_ast(&out.ast);
    let mut symptoms = Vec::new();
    env.check_unknowns(&out.ast, &mut symptoms);

    assert_eq!(
        type_mismatch_count(&symptoms),
        0,
        "CustomRange.new(1, 'hi') (no shared `x`) should accept mixed arg types; got: {:#?}",
        symptoms
    );
}

#[test]
fn range_new_body_tuple_satisfies_record_return_without_literal() {
    // Tuple-IS-record: just declaring the method (whose body is `(start, end)`
    // returned against the record `Range[x]`) should typecheck cleanly. No
    // diagnostic should be emitted for the tuple literal vs. record return.
    let src = RANGE_DECL.to_string();
    let out = parse_source_full(&src);
    let env = TyEnv::from_file_ast(&out.ast);
    let mut symptoms = Vec::new();
    env.check_unknowns(&out.ast, &mut symptoms);

    assert_eq!(
        type_mismatch_count(&symptoms),
        0,
        "Range.new body `(start, end)` should satisfy `Range[x]` via tuple-IS-record; got: {:#?}",
        symptoms
    );
}
