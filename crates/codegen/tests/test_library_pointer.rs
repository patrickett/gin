mod support;

#[test]
fn library_pointer_uses_address_load_store_and_gep_lowering() {
    let source = r#"Word is in 0 to 255
PointerToWord is @Word
read_addr(address PointerToWord) Word: address.(0)
replace(previous Word, next Word) Word: next
write_addr(address PointerToWord, value Word):
    pointer PointerToWord: address
    pointer.(0):: replace(pointer.(0), value)
return
move_addr(address PointerToWord, index Word) PointerToWord: @address.(index)
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
        "raw pointer library lowering should avoid anonymous-row/aggregate fallback ops: {mlir}"
    );
    assert!(
        !mlir.contains("llvm.insertvalue"),
        "raw pointer library lowering should avoid anonymous-row/aggregate fallback ops: {mlir}"
    );
}

#[test]
fn user_defined_generic_slice_uses_pointer_indexing_lowering() {
    let source = r#"Byte is in 0 to 255
Count is in 0 to 18446744073709551615
Slice(x) has pointer @x, length Count
read(value Slice(Byte), index Count) Byte:
    return value.pointer.(index)
"#;
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text(source, "user_slice_pointer.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("llvm.load"), "{mlir}");
    assert!(mlir.contains("!llvm.ptr"), "{mlir}");
    assert!(
        mlir.contains("llvm.extractvalue"),
        "user-defined slice should extract its pointer field from the nominal record: {mlir}"
    );
    assert!(
        !mlir.contains("llvm.insertvalue"),
        "slice indexing should not construct an aggregate: {mlir}"
    );
}
