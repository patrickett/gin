mod support;

#[test]
fn library_pointer_uses_address_load_store_and_gep_lowering() {
    let source = r#"Word is in 0...255
#default(RawPointer)
LibraryPointer(x) is @x
PointerToWord is LibraryPointer(Word)
#intrinsic(AddressLoad)
read_addr(address PointerToWord) Word extern
#intrinsic(AddressStore)
write_addr(address PointerToWord, value Word) extern
#intrinsic(AddressOffset)
move_addr(address PointerToWord, index Word) PointerToWord extern
entry(value Word) Word:
    address PointerToWord: @value
    moved PointerToWord: move_addr(address, 1)
    write_addr(moved, value)
    return read_addr(moved)
"#;
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "library_pointer.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("llvm.load"), "{mlir}");
    assert!(mlir.contains("llvm.store"), "{mlir}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("!llvm.ptr"), "{mlir}");
    assert!(
        !mlir.contains("llvm.extractvalue"),
        "raw pointer library lowering should avoid tuple/aggregate fallback ops: {mlir}"
    );
    assert!(
        !mlir.contains("llvm.insertvalue"),
        "raw pointer library lowering should avoid tuple/aggregate fallback ops: {mlir}"
    );
}
