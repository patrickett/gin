mod support;

#[test]
fn a_raw_pointer_to_an_array_does_not_claim_the_arrays_length_as_its_own_extent() {
    let source = r#"Byte is in 0 to 255
#fixed_array(element, count)
Buffer(element, count BigInt) has
get(pointer @Buffer(Byte, 4), index Byte) Buffer(Byte, 4): return pointer.[index]
"#;
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "raw_array.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(!mlir.contains("llvm.intr.trap"), "{mlir}");
    assert!(mlir.contains("-> !llvm.array<4 x i8>"), "{mlir}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
}

#[test]
fn a_zero_sized_value_can_be_accessed_at_a_raw_pointers_base() {
    let source = r#"Marker has
    finish(self):
        return
read(pointer @Marker) Marker: return pointer.[0]
borrow(pointer @Marker) ref Marker: return ref pointer.[0]
write(pointer @Marker, value own Marker):
    destination: pointer
    destination.[0]:: value
    return
"#;
    let (_, diagnostics) = support::codegen_to_mlir_text(source, "zero_sized_base.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}
