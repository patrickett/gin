mod support;

#[test]
fn proven_same_width_integer_conversion_emits_no_runtime_cast() {
    let source = "Tiny is in 0...15\nconverted: 10 as Tiny";
    let (mlir, symptoms) = support::codegen_to_mlir_text(source, "integer_conversion.gin", false);

    assert!(symptoms.is_empty(), "{symptoms:?}");
    assert!(mlir.contains("i4"), "{mlir}");
    for operation in ["arith.trunci", "arith.extsi", "arith.extui"] {
        assert!(!mlir.contains(operation), "unexpected {operation}:\n{mlir}");
    }
}
