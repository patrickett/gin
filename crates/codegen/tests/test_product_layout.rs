mod support;

#[test]
fn records_and_unit_use_product_mlir_and_aggregate_abi() {
    let source = r#"Byte is in 0 to 255
Word is in 0 to 18446744073709551615
Padded has first Byte, second Word
make(first Byte, second Word) Padded: return Padded(first, second)
identity(value Padded) Padded: return value
first(value Padded) Byte: return value.first
unit():
    return
external(value Padded) Padded: return value
PaddedPointer is @Padded
offset(base PaddedPointer, index Byte) PaddedPointer: return @base.[index]
entry(base PaddedPointer, index Byte) PaddedPointer: return offset(base, index)
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
    let source = format!(
        "{}Bool is True or False\npair() first Byte, second Byte: return 1, 2\nforward() first Byte, second Byte: return pair()\nfirst() Byte: return pair().first\nselected() Byte:\n    result: pair()\n    return result.second\nchoose(flag Bool) first Byte, second Byte:\n    when flag is True then\n        return pair()\n    return pair()\nchoose_when(flag Bool) first Byte, second Byte: return when flag is True then pair() else pair()\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    support::assert_valid_source(&source);
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text(&source, "result_row_values.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("llvm.insertvalue"), "{mlir}");
    assert!(mlir.contains("llvm.extractvalue"), "{mlir}");
    assert!(mlir.contains("function_type = () -> (i8, i8)"), "{mlir}");
    assert!(mlir.contains("callee = @pair"), "{mlir}");
    assert!(mlir.contains(": () -> (i8, i8)"), "{mlir}");
}

#[test]
fn named_result_rows_convert_to_the_llvm_abi() {
    let source = format!(
        "{}Bool is True or False\npair() first Byte, second Byte: return 1, 2\nforward() first Byte, second Byte: return pair()\nfirst() Byte: return pair().first\nselected() Byte:\n    result: pair()\n    return result.second\nchoose(flag Bool) first Byte, second Byte:\n    when flag is True then\n        return pair()\n    return pair()\nchoose_when(flag Bool) first Byte, second Byte: return when flag is True then pair() else pair()\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    support::assert_valid_source(&source);
    let (llvm, diagnostics) = support::codegen_to_llvm_ir(&source, "result_row_abi.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{llvm}");
    assert!(llvm.contains("define { i8, i8 } @pair()"), "{llvm}");
    assert!(llvm.contains("call { i8, i8 } @pair()"), "{llvm}");
}
