mod support;

#[test]
fn library_and_core_integers_use_the_same_bits_add_lowering() {
    let source = "\
CoreInt is in 0...4294967295
LibraryInt is in 0...4294967295

#intrinsic(BitsAdd)
core_add(a CoreInt, b CoreInt) CoreInt extern
#intrinsic(BitsAdd)
library_add(a LibraryInt, b LibraryInt) LibraryInt extern

core_sum(a CoreInt, b CoreInt) CoreInt: core_add(a, b)
library_sum(a LibraryInt, b LibraryInt) LibraryInt: library_add(a, b)
";
    let (mlir, symptoms) = support::codegen_to_mlir_text(source, "bits_add.gin", false);

    assert!(
        symptoms.is_empty(),
        "unexpected codegen symptoms: {symptoms:?}"
    );
    assert_eq!(mlir.matches("arith.addi").count(), 2, "{mlir}");
    assert!(mlir.matches("i32").count() >= 2, "{mlir}");
}

#[test]
fn integer_intrinsic_family_lowers_through_canonical_operations() {
    let source = "\
Byte is in 0...255
Word is in 0...65535

#intrinsic(BitsAdd)
add(a Byte, b Byte) Byte extern
#intrinsic(BitsSubtract)
subtract(a Byte, b Byte) Byte extern
#intrinsic(BitsMultiply)
multiply(a Byte, b Byte) Byte extern
#intrinsic(BitsSignedDivide)
signed_divide(a Byte, b Byte) Byte extern
#intrinsic(BitsUnsignedDivide)
unsigned_divide(a Byte, b Byte) Byte extern
#intrinsic(BitsSignedRemainder)
signed_remainder(a Byte, b Byte) Byte extern
#intrinsic(BitsUnsignedRemainder)
unsigned_remainder(a Byte, b Byte) Byte extern
#intrinsic(BitsAnd)
and_bits(a Byte, b Byte) Byte extern
#intrinsic(BitsOr)
or_bits(a Byte, b Byte) Byte extern
#intrinsic(BitsXor)
xor_bits(a Byte, b Byte) Byte extern
#intrinsic(BitsShiftLeft)
shift_left(a Byte, b Byte) Byte extern
#intrinsic(BitsShiftRightLogical)
shift_right_logical(a Byte, b Byte) Byte extern
#intrinsic(BitsShiftRightArithmetic)
shift_right_arithmetic(a Byte, b Byte) Byte extern
#intrinsic(BitsExtendSigned)
extend_signed(a Byte) Word extern
#intrinsic(BitsExtendUnsigned)
extend_unsigned(a Byte) Word extern
#intrinsic(BitsTruncate)
truncate(a Word) Byte extern

use_add(a Byte, b Byte) Byte: add(a, b)
use_subtract(a Byte, b Byte) Byte: subtract(a, b)
use_multiply(a Byte, b Byte) Byte: multiply(a, b)
use_signed_divide(a Byte, b Byte) Byte: signed_divide(a, b)
use_unsigned_divide(a Byte, b Byte) Byte: unsigned_divide(a, b)
use_signed_remainder(a Byte, b Byte) Byte: signed_remainder(a, b)
use_unsigned_remainder(a Byte, b Byte) Byte: unsigned_remainder(a, b)
use_and(a Byte, b Byte) Byte: and_bits(a, b)
use_or(a Byte, b Byte) Byte: or_bits(a, b)
use_xor(a Byte, b Byte) Byte: xor_bits(a, b)
use_shift_left(a Byte, b Byte) Byte: shift_left(a, b)
use_shift_right_logical(a Byte, b Byte) Byte: shift_right_logical(a, b)
use_shift_right_arithmetic(a Byte, b Byte) Byte: shift_right_arithmetic(a, b)
use_extend_signed(a Byte) Word: extend_signed(a)
use_extend_unsigned(a Byte) Word: extend_unsigned(a)
use_truncate(a Word) Byte: truncate(a)
";
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
