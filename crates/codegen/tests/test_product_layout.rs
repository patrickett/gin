mod support;

#[test]
fn records_and_unit_use_product_mlir_and_aggregate_abi() {
    let source = r#"Byte is in 0 to 255
Word is in 0 to 18446744073709551615
Padded has first Byte, second Word
make(first Byte, second Word) Padded: Padded(first, second)
identity(value Padded) Padded: value
first(value Padded) Byte: value.first
unit():
return
external(value Padded) Padded: value
PaddedPointer is @Padded
offset(base PaddedPointer, index Byte) PaddedPointer: @base.(index)
entry(base PaddedPointer, index Byte) PaddedPointer: offset(base, index)
"#;
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "product_layout.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("llvm.insertvalue"), "{mlir}");
    assert!(mlir.contains("llvm.extractvalue"), "{mlir}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("!llvm.struct"), "{mlir}");
    assert!(mlir.contains("sym_name = \"unit\""), "{mlir}");
    assert!(!mlir.contains("@unit() -> i64"), "{mlir}");
    assert!(mlir.contains("sym_name = \"external\""), "{mlir}");
}

#[test]
fn named_result_rows_are_distinct_ssa_results() {
    let source = r#"Byte is in 0 to 255
Bool is True or False
pair() first Byte, second Byte: return 1, 2
forward() first Byte, second Byte: return pair()
first() Byte: pair().first
selected() Byte:
    result: pair()
    return result.second
choose(flag Bool) first Byte, second Byte:
    if flag is True
        return pair()
    return
    return pair()
choose_when(flag Bool) first Byte, second Byte: when flag is True then pair() else pair()
"#;
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "result_row_values.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(!mlir.contains("llvm.insertvalue"), "{mlir}");
    assert!(!mlir.contains("llvm.extractvalue"), "{mlir}");
    assert!(mlir.contains("function_type = () -> (i8, i8)"), "{mlir}");
    assert!(mlir.contains("callee = @pair"), "{mlir}");
    assert!(mlir.contains(": () -> (i8, i8)"), "{mlir}");
}

#[test]
fn named_result_rows_convert_to_the_llvm_abi() {
    let source = r#"Byte is in 0 to 255
Bool is True or False
pair() first Byte, second Byte: return 1, 2
forward() first Byte, second Byte: return pair()
first() Byte: pair().first
selected() Byte:
    result: pair()
    return result.second
choose(flag Bool) first Byte, second Byte:
    if flag is True
        return pair()
    return
    return pair()
choose_when(flag Bool) first Byte, second Byte: when flag is True then pair() else pair()
"#;
    let (llvm, diagnostics) = support::codegen_to_llvm_ir(source, "result_row_abi.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{llvm}");
    assert!(llvm.contains("define { i8, i8 } @pair()"), "{llvm}");
    assert!(llvm.contains("call { i8, i8 } @pair()"), "{llvm}");
}
