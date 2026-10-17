mod support;

#[test]
fn total_integer_operators_lower_through_literal_infix_declarations() {
    let source = include_str!(
        "../../test-fixtures/fixtures/ordinary_integer_literal_infix_declarations.gin"
    );
    let (mlir, symptoms) = support::codegen_to_mlir_text(source, "operators.gin", false);

    assert!(symptoms.is_empty(), "{symptoms:?}");
    for operation in [
        "arith.addi",
        "arith.subi",
        "arith.muli",
        "arith.divui",
        "arith.remui",
        "arith.andi",
        "arith.ori",
        "arith.xori",
        "arith.shli",
        "arith.shrui",
    ] {
        assert!(mlir.contains(operation), "missing {operation}: {mlir}");
    }
}

#[test]
fn nominal_implementations_choose_distinct_operations_and_shift_interpretations() {
    let source = r#"Unsigned is in 0 to 255
Signed is in -128 to 127

unsigned_add_bits(a Unsigned, b Unsigned) Unsigned:
    left is in 0 to 255: a
    right is in 0 to 255: b
    return ((left + right) & 255) as Unsigned
signed_subtract_bits(a Signed, b Signed) Signed:
    left is in -128 to 127: a
    right is in -128 to 127: b
    bits is in 0 to 255: (left - right + 128) & 255
    return (bits - 128) as Signed
unsigned_shift_bits(a Unsigned, b Unsigned) Unsigned:
    left is in 0 to 255: a
    right is in 0 to 255: b
    return (left >> right) as Unsigned
signed_shift_bits(a Signed, b Signed and b >= 0) Signed:
    left is in -128 to 127: a
    right is in 0 to 127: b
    return (left >> right) as Signed

#infix('+')
unsigned_plus(a Unsigned, b Unsigned) Unsigned: return unsigned_add_bits(a, b)
#infix('+')
signed_plus(a Signed, b Signed) Signed: return signed_subtract_bits(a, b)
#infix('>>')
unsigned_right(a Unsigned, b Unsigned) Unsigned: return unsigned_shift_bits(a, b)
#infix('>>')
signed_right(a Signed, b Signed and b >= 0) Signed: return signed_shift_bits(a, b)

apply_unsigned_add(a Unsigned, b Unsigned) Unsigned: return a + b
apply_signed_add(a Signed, b Signed) Signed: return a + b
apply_unsigned_right(a Unsigned, b Unsigned) Unsigned: return a >> b
apply_signed_right(a Signed, b Signed and b >= 0) Signed: return a >> b
"#;
    let (mlir, symptoms) = support::codegen_to_mlir_text(source, "nominal_operators.gin", false);

    assert!(symptoms.is_empty(), "{symptoms:?}");
    for operation in ["arith.addi", "arith.subi", "arith.shrui", "arith.shrsi"] {
        assert!(mlir.contains(operation), "missing {operation}: {mlir}");
    }
}
