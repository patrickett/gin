mod support;

#[test]
fn payload_sum_uses_explicit_discriminant_and_payload_offsets() {
    let source = "\
#default(IntegerLiteral)
DefaultInteger is in 0...255
Maybe(value) is Some(value) or None
make(v DefaultInteger) Maybe(DefaultInteger): Maybe.Some(v)
";
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text_with_package_context(source, "sum_layout.gin");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("llvm.store"), "{mlir}");
    assert!(mlir.contains("llvm.load"), "{mlir}");
}

#[test]
fn payload_sum_pattern_projection_reads_the_authoritative_field_offset() {
    let source = "\
Bool is True or False
#default(IntegerLiteral)
DefaultInteger is in 0...255
Maybe(value) is Some(value) or None
has_value(v Maybe(DefaultInteger)) Bool: when v is Some(x) then True else False
";
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text_with_package_context(source, "sum_projection.gin");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("llvm.load"), "{mlir}");
    assert!(mlir.contains("scf.if"), "{mlir}");
}
