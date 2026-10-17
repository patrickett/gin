mod support;

#[test]
fn ordinary_main_has_its_declared_abi() {
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text("main:\n    return\n", "ordinary_main.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(!mlir.contains("__gin.entry"), "{mlir}");
    assert!(!mlir.contains("-> i64"), "{mlir}");
}

#[test]
fn entry_query_generates_host_adapter_for_any_callable_name() {
    let (mlir, diagnostics) = support::codegen_to_mlir_text(
        "Byte is in 0 to 255\n#entry\nlaunch() Byte: return 7\n",
        "entry.gin",
        false,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("@__gin.entry"), "{mlir}");
    assert!(
        mlir.contains("function_type = () -> i64, sym_name = \"main\""),
        "{mlir}"
    );
}

#[test]
fn entry_query_rejects_parameters_and_multiple_entries() {
    for source in [
        "Byte is in 0 to 255\n#entry\nlaunch(value Byte) Byte: return value\n",
        "#entry\nfirst:\n    return\n#entry\nsecond:\n    return\n",
    ] {
        let (verified, diagnostics) = support::codegen_verifies(source, "invalid_entry.gin");
        assert!(!verified);
        assert!(!diagnostics.is_empty());
    }
}
