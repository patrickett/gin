mod support;

#[test]
fn library_and_core_integers_use_the_same_bits_add_lowering() {
    let source = r#"CoreInt is in 0 to 4294967295
LibraryInt is in 0 to 4294967295

core_add(a CoreInt, b CoreInt) CoreInt:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
    return ((left + right) & 4294967295) as CoreInt
library_add(a LibraryInt, b LibraryInt) LibraryInt:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
    return ((left + right) & 4294967295) as LibraryInt

core_sum(a CoreInt, b CoreInt) CoreInt: return core_add(a, b)
library_sum(a LibraryInt, b LibraryInt) LibraryInt: return library_add(a, b)
"#;
    let (mlir, symptoms) = support::codegen_to_mlir_text(source, "bits_add.gin", false);

    assert!(
        symptoms.is_empty(),
        "unexpected codegen symptoms: {symptoms:?}"
    );
    assert_eq!(mlir.matches("arith.addi").count(), 2, "{mlir}");
    assert!(mlir.matches("i32").count() >= 2, "{mlir}");
}

#[test]
fn ordinary_integer_operations_lower_through_canonical_operations() {
    let source = include_str!("../../test-fixtures/fixtures/bits_add_ordinary_operations.gin");
    let (mlir, symptoms) = support::codegen_to_mlir_text(source, "bits_kernel.gin", false);

    assert!(symptoms.is_empty(), "{symptoms:?}");
    for operation in [
        "arith.addi",
        "arith.subi",
        "arith.muli",
        "arith.divsi",
        "arith.divui",
        "arith.remsi",
        "arith.remui",
        "arith.andi",
        "arith.ori",
        "arith.xori",
        "arith.shli",
        "arith.shrui",
        "arith.shrsi",
        "arith.extsi",
        "arith.extui",
        "arith.trunci",
    ] {
        assert!(mlir.contains(operation), "missing {operation}: {mlir}");
    }
}
