mod support;

#[test]
fn total_integer_operators_lower_through_literal_infix_declarations() {
    let source = r#"Word is in 0 to 4294967295

add_bits(a Word, b Word) Word:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
return (left + right) as Word
subtract_bits(a Word, b Word) Word:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
return (left - right) as Word
multiply_bits(a Word, b Word) Word:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
return (left * right) as Word
divide_bits(a Word, b Word) Word:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
return (left / right) as Word
modulo_bits(a Word, b Word) Word:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
return (left % right) as Word
and_bits(a Word, b Word) Word:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
return (left & right) as Word
or_bits(a Word, b Word) Word:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
return (left | right) as Word
xor_bits(a Word, b Word) Word:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
return (left ^ right) as Word
shift_left_bits(a Word, b Word) Word:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
return (left << right) as Word
shift_right_bits(a Word, b Word) Word:
    left is in 0 to 4294967295: a
    right is in 0 to 4294967295: b
return (left >> right) as Word

#infix('+')
renamed_plus(a Word, b Word) Word: add_bits(a, b)
#infix('-')
word_subtract(a Word, b Word) Word: subtract_bits(a, b)
#infix('*')
word_multiply(a Word, b Word) Word: multiply_bits(a, b)
#infix('/')
word_divide(a Word, b Word) Word: divide_bits(a, b)
#infix('%')
word_modulo(a Word, b Word) Word: modulo_bits(a, b)
#infix('&')
word_and(a Word, b Word) Word: and_bits(a, b)
#infix('|')
word_or(a Word, b Word) Word: or_bits(a, b)
#infix('^')
word_xor(a Word, b Word) Word: xor_bits(a, b)
#infix('<<')
word_shift_left(a Word, b Word) Word: shift_left_bits(a, b)
#infix('>>')
word_shift_right(a Word, b Word) Word: shift_right_bits(a, b)

add(a Word, b Word) Word: b
use_add(a Word, b Word) Word: a + b
use_subtract(a Word, b Word) Word: a - b
use_multiply(a Word, b Word) Word: a * b
use_divide(a Word, b Word) Word: a / b
use_modulo(a Word, b Word) Word: a % b
use_and(a Word, b Word) Word: a & b
use_or(a Word, b Word) Word: a | b
use_xor(a Word, b Word) Word: a ^ b
use_shift_left(a Word, b Word) Word: a << b
use_shift_right(a Word, b Word) Word: a >> b
"#;
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
return (left + right) as Unsigned
signed_subtract_bits(a Signed, b Signed) Signed:
    left is in -128 to 127: a
    right is in -128 to 127: b
return (left - right) as Signed
unsigned_shift_bits(a Unsigned, b Unsigned) Unsigned:
    left is in 0 to 255: a
    right is in 0 to 255: b
return (left >> right) as Unsigned
signed_shift_bits(a Signed, b Signed) Signed:
    left is in -128 to 127: a
    right is in -128 to 127: b
return (left >> right) as Signed

#infix('+')
unsigned_plus(a Unsigned, b Unsigned) Unsigned: unsigned_add_bits(a, b)
#infix('+')
signed_plus(a Signed, b Signed) Signed: signed_subtract_bits(a, b)
#infix('>>')
unsigned_right(a Unsigned, b Unsigned) Unsigned: unsigned_shift_bits(a, b)
#infix('>>')
signed_right(a Signed, b Signed) Signed: signed_shift_bits(a, b)

apply_unsigned_add(a Unsigned, b Unsigned) Unsigned: a + b
apply_signed_add(a Signed, b Signed) Signed: a + b
apply_unsigned_right(a Unsigned, b Unsigned) Unsigned: a >> b
apply_signed_right(a Signed, b Signed) Signed: a >> b
"#;
    let (mlir, symptoms) = support::codegen_to_mlir_text(source, "nominal_operators.gin", false);

    assert!(symptoms.is_empty(), "{symptoms:?}");
    for operation in ["arith.addi", "arith.subi", "arith.shrui", "arith.shrsi"] {
        assert!(mlir.contains(operation), "missing {operation}: {mlir}");
    }
}
