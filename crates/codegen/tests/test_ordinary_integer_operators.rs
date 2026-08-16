mod support;

#[test]
fn total_integer_operators_lower_through_role_registered_declarations() {
    let source = "\
Word is in 0...4294967295

#intrinsic(BitsAdd)
add_bits(a Word, b Word) Word extern
#intrinsic(BitsSubtract)
subtract_bits(a Word, b Word) Word extern
#intrinsic(BitsMultiply)
multiply_bits(a Word, b Word) Word extern
#intrinsic(BitsUnsignedDivide)
divide_bits(a Word, b Word) Word extern
#intrinsic(BitsUnsignedRemainder)
modulo_bits(a Word, b Word) Word extern
#intrinsic(BitsAnd)
and_bits(a Word, b Word) Word extern
#intrinsic(BitsOr)
or_bits(a Word, b Word) Word extern
#intrinsic(BitsXor)
xor_bits(a Word, b Word) Word extern
#intrinsic(BitsShiftLeft)
shift_left_bits(a Word, b Word) Word extern
#intrinsic(BitsShiftRightLogical)
shift_right_bits(a Word, b Word) Word extern

#operator(Add)
renamed_plus(a Word, b Word) Word: add_bits(a, b)
#operator(Subtract)
word_subtract(a Word, b Word) Word: subtract_bits(a, b)
#operator(Multiply)
word_multiply(a Word, b Word) Word: multiply_bits(a, b)
#operator(Divide)
word_divide(a Word, b Word) Word: divide_bits(a, b)
#operator(Modulo)
word_modulo(a Word, b Word) Word: modulo_bits(a, b)
#operator(BitAnd)
word_and(a Word, b Word) Word: and_bits(a, b)
#operator(BitOr)
word_or(a Word, b Word) Word: or_bits(a, b)
#operator(BitXor)
word_xor(a Word, b Word) Word: xor_bits(a, b)
#operator(ShiftLeft)
word_shift_left(a Word, b Word) Word: shift_left_bits(a, b)
#operator(ShiftRight)
word_shift_right(a Word, b Word) Word: shift_right_bits(a, b)

Add(a Word, b Word) Word: a
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
";
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
    let source = "\
Unsigned is in 0...255
Signed is in -128...127

#intrinsic(BitsAdd)
unsigned_add_bits(a Unsigned, b Unsigned) Unsigned extern
#intrinsic(BitsSubtract)
signed_subtract_bits(a Signed, b Signed) Signed extern
#intrinsic(BitsShiftRightLogical)
unsigned_shift_bits(a Unsigned, b Unsigned) Unsigned extern
#intrinsic(BitsShiftRightArithmetic)
signed_shift_bits(a Signed, b Signed) Signed extern

#operator(Add)
unsigned_plus(a Unsigned, b Unsigned) Unsigned: unsigned_add_bits(a, b)
#operator(Add)
signed_plus(a Signed, b Signed) Signed: signed_subtract_bits(a, b)
#operator(ShiftRight)
unsigned_right(a Unsigned, b Unsigned) Unsigned: unsigned_shift_bits(a, b)
#operator(ShiftRight)
signed_right(a Signed, b Signed) Signed: signed_shift_bits(a, b)

apply_unsigned_add(a Unsigned, b Unsigned) Unsigned: a + b
apply_signed_add(a Signed, b Signed) Signed: a + b
apply_unsigned_right(a Unsigned, b Unsigned) Unsigned: a >> b
apply_signed_right(a Signed, b Signed) Signed: a >> b
";
    let (mlir, symptoms) = support::codegen_to_mlir_text(source, "nominal_operators.gin", false);

    assert!(symptoms.is_empty(), "{symptoms:?}");
    for operation in ["arith.addi", "arith.subi", "arith.shrui", "arith.shrsi"] {
        assert!(mlir.contains(operation), "missing {operation}: {mlir}");
    }
}
