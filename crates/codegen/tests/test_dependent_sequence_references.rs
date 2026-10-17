mod support;

const PRELUDE: &str = r#"
Byte is in 0 to 255
Byte has finish(self): return
Length is in 0 to 255
Length has finish(self): return
Slice(element) has
    length Length
    data ref [element; length]
    finish(self):
        return
"#;

fn lower(source: &str) -> String {
    let source = format!("{PRELUDE}{source}");
    support::assert_valid_source(&source);
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text(&source, "dependent_sequence.gin", false);
    assert!(
        diagnostics.is_empty(),
        r#"
{diagnostics:?}
{mlir}"#
    );
    mlir
}

#[test]
fn indexing_uses_runtime_length_and_element_layout() {
    let mlir = lower(
        r#"
peek(n Length, source ref [Byte; n], index Length) Byte: return source.[index]
"#,
    );
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("llvm.intr.trap"), "{mlir}");
}

#[test]
fn record_storage_indexing_uses_its_sibling_length() {
    let mlir = lower(
        r#"
peek(slice ref Slice(Byte), index Length) Byte: return slice.data.[index]
"#,
    );
    assert!(mlir.contains("llvm.extractvalue"), "{mlir}");
    assert!(mlir.contains("llvm.intr.trap"), "{mlir}");
}

#[test]
fn runtime_length_reference_lowers_a_counted_loop() {
    let mlir = lower(
        r#"
visit(n Length, source ref [Byte; n]):
    for value in source
        observed is in Byte: value
    return
"#,
    );
    assert!(mlir.contains("scf.while"), "{mlir}");
}
