mod support;

#[test]
fn integer_equality_lowers_to_i1_comparison() {
    let source = "\
Bool is True or False
Int is in 0...4294967295
same := 1 = 2
";
    let (mlir, symptoms) = support::codegen_to_mlir_text(source, "equality.gin", false);

    assert!(symptoms.is_empty(), "{symptoms:?}");
    assert!(mlir.contains("\"arith.cmpi\""), "{mlir}");
    assert!(mlir.contains("predicate = 0 : i64"), "{mlir}");
    let (verified, symptoms) = support::codegen_verifies(source, "equality.gin");
    assert!(symptoms.is_empty(), "{symptoms:?}");
    assert!(verified);
}

#[test]
fn lowercase_pattern_binding_reuses_the_subject_value() {
    let source = "\
Bool is True or False
identity(value Bool) Bool: when value is bound then bound
";
    let (verified, symptoms) = support::codegen_verifies(source, "pattern_binding.gin");

    assert!(symptoms.is_empty(), "{symptoms:?}");
    assert!(verified);
}
