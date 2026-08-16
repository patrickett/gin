mod support;

use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler};
use parser::parse_from_str;
use typecheck::{FileId, transform::transform_file};

fn assert_range_materializes(min: &str, max: &str, value: &str, width: u32) {
    let source = format!("Window is in {min}...{max}\nvalue Window: {value}\n");
    let (mlir, symptoms) = support::codegen_to_mlir_text(&source, "integer_width.gin", false);
    assert!(symptoms.is_empty(), "{symptoms:?}");
    assert!(mlir.contains(&format!("i{width}")), "{mlir}");
}

#[test]
fn targetless_library_cannot_be_codegened() {
    let source = "Window is in 0...255\nvalue Window: 1\n";
    let typed = transform_file(parse_from_str(source), FileId(0));
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("targetless.gin", source, &typed.span_table),
    );
    assert!(module.is_none());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code.slug(), "unsupported-target-layout");
}

#[test]
fn range_0_to_1_materializes_as_i1() {
    assert_range_materializes("0", "1", "1", 1);
}

#[test]
fn range_0_to_2_materializes_as_i2() {
    assert_range_materializes("0", "2", "2", 2);
}

#[test]
fn range_0_to_255_materializes_as_i8() {
    assert_range_materializes("0", "255", "255", 8);
}

#[test]
fn range_0_to_256_materializes_as_i9() {
    assert_range_materializes("0", "256", "256", 9);
}

#[test]
fn range_250_to_260_materializes_as_i9() {
    assert_range_materializes("250", "260", "260", 9);
}

#[test]
fn range_negative_1_to_0_materializes_as_i1() {
    assert_range_materializes("-1", "0", "-1", 1);
}

#[test]
fn range_negative_1_to_1_materializes_as_i2() {
    assert_range_materializes("-1", "1", "1", 2);
}

#[test]
fn range_negative_1_to_127_materializes_as_i8() {
    assert_range_materializes("-1", "127", "127", 8);
}

#[test]
fn range_negative_1_to_128_materializes_as_i9() {
    assert_range_materializes("-1", "128", "128", 9);
}

#[test]
fn range_negative_1_to_255_materializes_as_i9() {
    assert_range_materializes("-1", "255", "255", 9);
}

#[test]
fn range_negative_1_to_256_materializes_as_i10() {
    assert_range_materializes("-1", "256", "256", 10);
}

#[test]
fn inferred_integer_and_comparison_ir_contains_expected_shapes() {
    let source = "\
Bool is True or False
I1 is in 0...1
I2 is in 0...3
I9 is in 0...256
I128 is in -170141183460469231731687303715884105728...170141183460469231731687303715884105727
Signed128 is in -128...127

Decision is Enabled or Disabled
Unsigned is in 0...255
Signed is in -128...127

#intrinsic(BitsUnsignedLess)
unsigned_less_bits(a Unsigned, b Unsigned) Decision extern
#operator(Less)
unsigned_less(a Unsigned, b Unsigned) Decision: unsigned_less_bits(a, b)

#intrinsic(BitsSignedGreaterOrEqual)
signed_greater_equal_bits(a Signed, b Signed) Decision extern
#operator(GreaterOrEqual)
signed_greater_equal(a Signed, b Signed) Decision: signed_greater_equal_bits(a, b)

SmallPair has left I2, right I9

lit_i1 I1: 1
lit_i2 I2: 2
lit_i9 I9: 256
lit_i128 I128: 1

compare_unsigned(a Unsigned, b Unsigned) Decision: a < b
compare_signed(a Signed, b Signed) Decision: a >= b
mk_pair(a I2, b I9) SmallPair: SmallPair(left: a, right: b)
pair: mk_pair(1, 256)
left_field: pair.left
right_field: pair.right
";
    let (mlir, symptoms) = support::codegen_to_mlir_text(source, "integer_matrix.gin", false);

    assert!(symptoms.is_empty(), "{symptoms:?}");
    assert!(mlir.contains(" : i1"), "{mlir}");
    assert!(mlir.contains(" : i2"), "{mlir}");
    assert!(mlir.contains(" : i9"), "{mlir}");
    assert!(mlir.contains(" : i128"), "{mlir}");
    assert!(mlir.contains("\"arith.cmpi\""), "{mlir}");
    assert!(mlir.contains("predicate = 6 : i64"), "{mlir}");
    assert!(mlir.contains("predicate = 5 : i64"), "{mlir}");
    assert!(mlir.contains("llvm.insertvalue"), "{mlir}");
    assert!(mlir.contains("llvm.extractvalue"), "{mlir}");
}
