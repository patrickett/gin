mod support;

#[test]
fn text_slice_projects_pointer_and_byte_length_without_runtime_helpers() {
    let source = r#"ViewByte is in 0 to 255
ViewLength is in 0 to 18446744073709551615
Slice(element) has ptr @element, len ViewLength
Byte is in 0 to 255
Size is in 0 to 18446744073709551615
pointer(text Slice(ViewByte)) @Byte: text.ptr as @Byte
length(text Slice(ViewByte)) Size: text.len as Size
view(pointer @Byte, length Size) Slice(ViewByte): Slice(ptr: pointer as @ViewByte, len: length as ViewLength)
"#;
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "text.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("llvm.extractvalue"), "{mlir}");
    assert!(!mlir.contains("func.call"), "{mlir}");
    assert!(!mlir.contains("llvm.call"), "{mlir}");
}
