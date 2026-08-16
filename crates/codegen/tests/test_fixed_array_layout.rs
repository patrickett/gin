mod support;

#[test]
fn library_defined_fixed_array_lowers_as_an_inline_aggregate() {
    let source = r#"#default(Integer)
Byte is in 0...255
#fixed_array(element, count)
Buffer(element Type, count BigInt) has
make(value Byte) Buffer(Byte, 4): (value; 4)
get(value Buffer(Byte, 4), index Byte) Byte: value.(index)
set(mut value Buffer(Byte, 4), index Byte, replacement Byte): value.(index):: replacement
"#;
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "fixed_array.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("llvm.array"), "{mlir}");
    assert!(mlir.contains("llvm.insertvalue"), "{mlir}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("llvm.load"), "{mlir}");
    assert!(
        mlir.contains("function_type = (i8) -> !llvm.array<4 x i8>"),
        "{mlir}"
    );
}

#[test]
fn symbolic_fixed_array_lengths_in_declaration_reach_codegen_boundary_as_error() {
    let source = r#"Int is in 0...255
Byte is in 0...255
#fixed_array(element, count)
Buffer(element Type, count Int) has
make(value Byte): Buffer(Byte, n): (value; n)
"#;
    let (_verified, symptoms) = support::codegen_verifies(source, "symbolic_fixed_array.gin");
    assert!(
        symptoms.iter().any(|diagnostic| {
            diagnostic.code.slug() == "unresolved-fixed-array-construction"
                && diagnostic.message.contains(
                    "fixed-array construction reached codegen without array representation",
                )
        }),
        "expected unresolved fixed-array construction symptom: {:?}",
        symptoms
    );

    let (mlir, _) = support::codegen_to_mlir_text(source, "symbolic_fixed_array.gin", false);
    assert!(
        !mlir.contains("llvm.array"),
        "symbolic fixed-array construction should not lower to concrete array: {mlir}"
    );
}

#[test]
fn symbolic_fixed_array_length_substitution_preserves_derived_array_layout() {
    let source = r#"Int is in 0...255
Byte is in 0...255
#fixed_array(element, count)
Buffer(element Type, count Int) has
make(value Byte): Buffer(Byte, n + 1): (value; n + 1)
"#;
    let (_verified, symptoms) =
        support::codegen_verifies(source, "symbolic_fixed_array_substitution.gin");
    assert!(
        symptoms.iter().any(|diagnostic| {
            diagnostic.code.slug() == "unresolved-fixed-array-construction"
                && diagnostic.message.contains(
                    "fixed-array construction reached codegen without array representation",
                )
        }),
        "expected unresolved fixed-array construction symptom: {:?}",
        symptoms
    );
    let (mlir, _symptoms) =
        support::codegen_to_mlir_text(source, "symbolic_fixed_array_substitution.gin", false);

    assert!(
        !mlir.contains("llvm.array"),
        "symbolic fixed-array construction with n+1 should still be blocked without concrete substitution"
    );
}

#[test]
fn symbolic_fixed_array_length_with_concrete_substitution_lowers_to_array() {
    let source = r#"Int is in 0...255
Byte is in 0...255
#fixed_array(element, count)
Buffer(element Type, count Int) has
make(value Byte): Buffer(Byte, 3): (value; 3)
main() Buffer(Byte, 3):
    return Buffer.make(1)
"#;
    let (mlir, symptoms) = support::codegen_to_mlir_text(
        source,
        "symbolic_fixed_array_concrete_substitution.gin",
        false,
    );
    assert!(
        symptoms.is_empty(),
        "expected concrete array substitution to compile: {symptoms:?}"
    );
    assert!(
        mlir.contains("llvm.array"),
        "concrete fixed-array construction should lower to concrete llvm.array: {mlir}"
    );
}

#[test]
fn compound_fixed_array_write_evaluates_the_index_once() {
    let source = r#"#default(Integer)
Byte is in 0...255
#fixed_array(element, count)
Buffer(element Type, count BigInt) has
#intrinsic(BitsAdd)
add_bits(a Byte, b Byte) Byte extern
#operator(Add)
#inline
byte_add(a Byte, b Byte) Byte: add_bits(a, b)
next_index() Byte: 0
update(mut value Buffer(Byte, 4), replacement Byte) Byte:
    value.(next_index())+: replacement
    return replacement
"#;
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text(source, "compound_fixed_array.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert_eq!(mlir.matches("callee = @next_index").count(), 1, "{mlir}");
    assert!(mlir.contains("callee = @byte_add"), "{mlir}");
}
