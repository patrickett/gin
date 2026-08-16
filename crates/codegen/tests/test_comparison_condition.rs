mod support;

#[test]
fn comparison_intrinsics_lower_predicate_and_widen_result() {
    let source = "\
Unsigned is in 0...255
Signed is in -128...127
Decision is in 0...255

#intrinsic(BitsUnsignedLess)
unsigned_less_bits(a Unsigned, b Unsigned) Decision extern
#intrinsic(BitsSignedGreaterOrEqual)
signed_greater_equal_bits(a Signed, b Signed) Decision extern

#operator(Less)
renamed_unsigned_comparison(a Unsigned, b Unsigned) Decision: unsigned_less_bits(a, b)
#operator(GreaterOrEqual)
renamed_signed_comparison(a Signed, b Signed) Decision: signed_greater_equal_bits(a, b)

lt(a Unsigned, b Unsigned) Decision: a
use_unsigned(a Unsigned, b Unsigned) Decision: a < b
use_signed(a Signed, b Signed) Decision: a >= b
";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "comparison.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("\"arith.cmpi\""), "{mlir}");
    assert!(
        mlir.contains("predicate = 6 : i64"),
        "unsigned less missing: {mlir}"
    );
    assert!(
        mlir.contains("predicate = 5 : i64"),
        "signed greater-equal missing: {mlir}"
    );
    assert!(
        mlir.matches("\"arith.extui\"").count() == 2 && mlir.contains(": (i1) -> i8"),
        "comparison results should widen from the one-bit sum discriminant to the declared result width: {mlir}"
    );
}

#[test]
fn explicit_pattern_condition_drives_runtime_branching() {
    let source = "\
Flag is Enabled or Disabled
choose(value Flag) Flag: when value is Flag.Enabled then Flag.Enabled else Flag.Disabled
";
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text_with_package_context(source, "condition.gin");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("\"arith.cmpi\""), "{mlir}");
    assert!(mlir.contains("\"scf.if\""), "{mlir}");
}

#[test]
fn structural_and_condition_lowers_with_short_circuit_region() {
    let source = "\
choose: when 1 < 2 is True and 2 < 3 is True then 1 else 0
";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "and-condition.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.matches("\"scf.if\"").count() >= 2, "{mlir}");
    assert!(mlir.matches("\"arith.cmpi\"").count() >= 2, "{mlir}");
}

#[test]
fn structural_or_condition_lowers_with_short_circuit_region() {
    let source = "\
choose: when 2 < 1 is True or 2 < 3 is True then 1 else 0
";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "or-condition.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.matches("\"scf.if\"").count() >= 2, "{mlir}");
    assert!(mlir.matches("\"arith.cmpi\"").count() >= 2, "{mlir}");
}

#[test]
fn structural_not_condition_lowers_as_i1_negation() {
    let source = "\
choose: when not (2 < 1 is True) then 1 else 0
";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "not-condition.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("arith.xori"), "{mlir}");
}
