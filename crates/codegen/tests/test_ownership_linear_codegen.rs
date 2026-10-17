mod support;

#[test]
fn rematerialized_conversions_emit_constants_without_ownership_helpers() {
    let source = "Tiny is in 0 to 7\nmain:\n    value: 7\n    first: value as Tiny\n    second: value as Tiny\n    return 0";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "rematerialize.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.matches("value = -1 : i3").count() >= 3, "{mlir}");
    assert!(
        !mlir.contains("copy")
            && !mlir.contains("memcpy")
            && !mlir.contains("memmove")
            && !mlir.contains("destroy")
            && !mlir.contains("drop")
            && !mlir.contains("release"),
        "{mlir}"
    );
}

#[test]
fn test_linear_ownership_paths_do_not_emit_copy_or_drop_helpers() {
    let source = "Int is in 1 to 400\nInt has\n    finish(self):\n        return\n\nIntPair has\n    a Int\n    b Int\n    finish(self):\n        return\n\nconsume_pair(value IntPair) Int: return 1\n\nmain:\n    pair: IntPair(a: 1, b: 2)\n    consume_pair(pair)\n    return\n";
    let parsed = parser::parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let typed = typecheck::transform::transform_file(parsed, typecheck::FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let (mlir_text, diagnostics) = support::codegen_to_mlir_text(source, "ownership.gin", false);
    assert!(
        diagnostics.is_empty(),
        "linear ownership code should not emit codegen diagnostics: {:?}",
        diagnostics
    );
    assert!(
        !mlir_text.contains("copy")
            && !mlir_text.contains("memcpy")
            && !mlir_text.contains("memmove"),
        "MLIR should not contain explicit copy helpers for linear ownership values\n{mlir_text}"
    );
    assert!(
        !mlir_text.contains("destroy")
            && !mlir_text.contains("drop")
            && !mlir_text.contains("release"),
        "MLIR should not contain explicit destructor helpers for linear ownership values\n{mlir_text}"
    );
}

#[test]
fn test_inferred_integer_backend_matrix_contains_expected_widths_and_predicates() {
    let source = r#"I1 is in 0 to 1
I2 is in 0 to 2
I9 is in 0 to 256
I128 is in -170141183460469231731687303715884105728 to 170141183460469231731687303715884105727
Signed128 is in -128 to 127
Bool is False or True
Decision is Lower or Upper
I1 has
    finish(self):
        return
I2 has
    finish(self):
        return
I9 has
    finish(self):
        return
I128 has
    finish(self):
        return
Signed128 has
    finish(self):
        return


unsigned_less_bits(a I2, b I2) I1:
    left is in 0 to 2: a
    right is in 0 to 2: b
    return when left < right then 1 else 0
signed_less_bits(a Signed128, b Signed128) I1:
    left is in -128 to 127: a
    right is in -128 to 127: b
    return when left < right then 1 else 0

main:
    i1_zero I1: 0
    i1_one I1: 1
    i2_zero I2: 0
    i2_one I2: 1
    i2_value I2: 2
    signed_min Signed128: -128
    signed_max Signed128: 127
    i9_value I9: 256
    i128_value I128: 1

    compare_u1: 0 < 1
    compare_u2: 0 < 2
    compare_u9: 0 < 256
    compare_unsigned: unsigned_less_bits(i2_zero, i2_value)
    compare_signed: signed_less_bits(signed_min, signed_max)

    comparison_family: Decision: when compare_u2 then Decision.Lower else Decision.Upper
    sum_projection: when comparison_family is Decision.Lower then True else False
    return sum_projection
"#;

    let parsed = parser::parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let typed = typecheck::transform::transform_file(parsed, typecheck::FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());

    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "backend_matrix.gin", false);

    assert!(
        diagnostics.is_empty(),
        "backend matrix diagnostics: {:?}",
        diagnostics
    );
    assert!(mlir.contains("i1"), "missing i1 lowering\n{mlir}");
    assert!(mlir.contains("i2"), "missing i2 lowering\n{mlir}");
    assert!(mlir.contains("i9"), "missing i9 lowering\n{mlir}");
    assert!(mlir.contains("i128"), "missing i128 lowering\n{mlir}");

    assert!(
        mlir.contains("predicate = 6 : i64"),
        "missing unsigned predicate\n{mlir}"
    );
    assert!(
        mlir.contains("predicate = 2 : i64"),
        "missing signed predicate\n{mlir}"
    );

    assert!(
        mlir.contains("function_type = (i2, i2) -> i1"),
        "two-alternative results should retain their one-bit representation\n{mlir}"
    );
    assert!(
        !mlir.contains("llvm.alloca")
            && !mlir.contains("llvm.store")
            && !mlir.contains("llvm.load"),
        "initialized locals should remain SSA values\n{mlir}"
    );
}

#[test]
fn inferred_integer_widths_and_comparisons_reach_exact_llvm_forms() {
    let source = r#"I1 is in 0 to 1
I2 is in 0 to 2
I9 is in 0 to 256
I128 is in -170141183460469231731687303715884105728 to 170141183460469231731687303715884105727
Signed is in -128 to 127
keep_i1(value I1) I1: return value
keep_i2(value I2) I2: return value
keep_i9(value I9) I9: return value
keep_i128(value I128) I128: return value
unsigned_less(a I2, b I2) False or True:
    left is in 0 to 2: a
    right is in 0 to 2: b
    return left < right
signed_less(a Signed, b Signed) False or True:
    left is in -128 to 127: a
    right is in -128 to 127: b
    return left < right"#;
    let (llvm, diagnostics) = support::codegen_to_llvm_ir(source, "exact_backend.gin");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for signature in [
        "define i1 @keep_i1(i1",
        "define i2 @keep_i2(i2",
        "define i9 @keep_i9(i9",
        "define i128 @keep_i128(i128",
        "define i1 @unsigned_less(i2 %0, i2 %1)",
        "define i1 @signed_less(i8 %0, i8 %1)",
    ] {
        assert!(llvm.contains(signature), "missing `{signature}`:\n{llvm}");
    }
    assert!(
        !llvm.contains("alloca")
            && !llvm.contains(" load ")
            && !llvm.contains(" store ")
            && !llvm.contains("Bool")
            && !llvm.contains("copy")
            && !llvm.contains("drop"),
        "unexpected legacy or memory artifact:\n{llvm}"
    );
}

#[test]
fn straight_line_rebind_replaces_the_ssa_place_version() {
    let source = "Byte is in 0 to 255\nmain:\n    value Byte: 1\n    value :: 2\n    value";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "ssa_rebind.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("value = 1 : i8"), "{mlir}");
    assert!(mlir.contains("value = 2 : i8"), "{mlir}");
    assert!(
        !mlir.contains("llvm.alloca")
            && !mlir.contains("llvm.store")
            && !mlir.contains("llvm.load"),
        "straight-line `::` should replace an SSA binding without memory traffic\n{mlir}"
    );
}

#[test]
fn conditional_rebind_lowers_the_place_join_as_an_scf_result() {
    let source = "Byte is in 0 to 255\nmain(condition Byte) Byte:\n    value Byte: 1\n    when condition < 2 then\n        value :: 2\n    return value";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "ssa_join.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("scf.if"), "{mlir}");
    assert!(mlir.contains(": (i1) -> i8"), "{mlir}");
    assert!(mlir.contains("value = 1 : i8"), "{mlir}");
    assert!(mlir.contains("value = 2 : i8"), "{mlir}");
    assert!(
        !mlir.contains("llvm.alloca")
            && !mlir.contains("llvm.store")
            && !mlir.contains("llvm.load"),
        "conditional `::` should lower through an SSA join result\n{mlir}"
    );
}

#[test]
fn multi_arm_rebind_lowers_the_place_join_as_a_when_result() {
    let source = "Byte is in 0 to 255\nFlag is Left or Right\nmain(condition Flag) Byte:\n    value Byte: 1\n    choice Byte: when condition is\n        Flag.Left then value :: 2\n                  else value :: 3\n    return value";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "ssa_when_join.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains(": (i1) -> (i8, i8)"), "{mlir}");
    assert!(mlir.contains("value = 2 : i8"), "{mlir}");
    assert!(mlir.contains("value = 3 : i8"), "{mlir}");
    assert!(
        !mlir.contains("llvm.alloca")
            && !mlir.contains("llvm.store")
            && !mlir.contains("llvm.load"),
        "multi-arm `::` should lower through SSA results\n{mlir}"
    );
}

#[test]
fn loop_rebind_lowers_the_place_version_as_a_while_iteration_argument() {
    let source = "Small is in 0 to 2\nFlag is True or False\nmain(flag Flag) Small:\n    value Small: 0\n    while flag is Flag.True do\n        value :: 1\n    return value";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "ssa_loop_phi.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("scf.while"), "{mlir}");
    assert!(mlir.contains(": (i2) -> i2"), "{mlir}");
    assert!(
        !mlir.contains("llvm.alloca")
            && !mlir.contains("llvm.store")
            && !mlir.contains("llvm.load"),
        "loop `::` should lower through a loop-carried SSA value\n{mlir}"
    );
}

#[test]
#[allow(unsafe_code)]
fn for_rebind_lowers_the_place_version_as_an_iteration_argument() {
    let source = "Small is in 0 to 3\nSmall has\n    finish(self):\n        return\nreplace(previous Small, next Small) Small: return next\n#entry\nmain() Small:\n    value Small: 0\n    for index in 0 to 2\n        value :: replace(value, index as Small)\n    return value";
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    let (mlir, diagnostics) =
        support::codegen_assembly_to_mlir_text_for_target(source, "ssa_for_phi.gin", target);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("scf.while"), "{mlir}");
    assert!(mlir.contains(": (i2, i1, i2) -> (i2, i1, i2)"), "{mlir}");
    assert!(
        !mlir.contains("llvm.alloca")
            && !mlir.contains("llvm.store")
            && !mlir.contains("llvm.load"),
        "for-loop `::` should lower through a loop-carried SSA value\n{mlir}"
    );
    let context = codegen::NativeCompiler::create_context();
    let module = melior::ir::Module::parse(&context, &mlir).unwrap();
    let engine = codegen::NativeEngine::new(module, "main").unwrap();
    assert_eq!(unsafe { engine.invoke_i64() }.unwrap(), 2);
}

#[test]
fn fixed_array_for_loop_unrolls_structural_elements() {
    let source = "main:\n    for value in [1, 2, 3]\n        iteration: 0\n";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "array_for.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(
        !mlir.contains("fixed-array iteration is not lowered yet"),
        "{mlir}"
    );
}
