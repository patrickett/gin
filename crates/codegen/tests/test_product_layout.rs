mod support;

#[test]
fn records_and_unit_use_product_mlir_and_aggregate_abi() {
    let source = r#"Byte is in 0...255
Word is in 0...18446744073709551615
Padded has first Byte, second Word
make(first Byte, second Word) Padded: Padded(first, second)
identity(value Padded) Padded: value
first(value Padded) Byte: value.first
unit():
return
external(value Padded) Padded extern
LibraryPointer(x) is @x
PaddedPointer is LibraryPointer(Padded)
#intrinsic(AddressOffset)
offset(base PaddedPointer, index Byte) PaddedPointer extern
entry(base PaddedPointer, index Byte) PaddedPointer: offset(base, index)
"#;
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "product_layout.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("llvm.insertvalue"), "{mlir}");
    assert!(mlir.contains("llvm.extractvalue"), "{mlir}");
    assert!(mlir.contains("arith.muli"), "{mlir}");
    assert!(mlir.contains("!llvm.struct"), "{mlir}");
    assert!(mlir.contains("sym_name = \"unit\""), "{mlir}");
    assert!(!mlir.contains("@unit() -> i64"), "{mlir}");
    assert!(mlir.contains("sym_name = \"external\""), "{mlir}");
}
