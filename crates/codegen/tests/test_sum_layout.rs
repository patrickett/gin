mod support;

#[test]
fn payload_sum_uses_explicit_discriminant_and_payload_offsets() {
    let source = "DefaultInteger is in 0 to 255\nMaybe(value) is Some(value) or None\nmake(v DefaultInteger) Maybe(DefaultInteger): return Maybe.Some(v)\n";
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text_with_package_context(source, "sum_layout.gin");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("llvm.store"), "{mlir}");
    assert!(mlir.contains("llvm.load"), "{mlir}");
}

#[test]
fn payload_sum_pattern_projection_reads_the_authoritative_field_offset() {
    let source = "Bool is True or False\nDefaultInteger is in 0 to 255\nMaybe(value) is Some(value) or None\nhas_value(v Maybe(DefaultInteger)) Bool: return when v is Some(x) then True else False\n";
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text_with_package_context(source, "sum_projection.gin");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("llvm.load"), "{mlir}");
    assert!(mlir.contains("scf.if"), "{mlir}");
}
