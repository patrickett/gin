mod support;

#[test]
fn comparison_operators_lower_signed_and_unsigned_predicates() {
    let source = r#"Unsigned is in 0 to 255
Signed is in -128 to 127

unsigned_less_bits(a Unsigned, b Unsigned) False or True:
    left is in 0 to 255: a
    right is in 0 to 255: b
return left < right
signed_greater_equal_bits(a Signed, b Signed) False or True:
    left is in -128 to 127: a
    right is in -128 to 127: b
return left >= right

#infix('<')
renamed_unsigned_comparison(a Unsigned, b Unsigned) False or True: unsigned_less_bits(a, b)
#infix('>=')
renamed_signed_comparison(a Signed, b Signed) False or True: signed_greater_equal_bits(a, b)

use_unsigned(a Unsigned, b Unsigned) False or True: a < b
use_signed(a Signed, b Signed) False or True: a >= b
"#;
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "comparison.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("\"arith.cmpi\""), "{mlir}");
    assert!(
        mlir.contains("predicate = 6 : i64"),
        "unsigned less missing: {mlir}"
    );
    assert!(
        mlir.contains("predicate = 5 : i64"),
        "signed greater-equal missing: {mlir}"
    );
    assert!(mlir.contains("-> i1"), "{mlir}");
}

#[test]
fn explicit_pattern_condition_drives_runtime_branching() {
    let source = "\
Flag is Enabled or Disabled
choose(value Flag) Flag: when value is Flag.Enabled then Flag.Enabled else Flag.Disabled
";
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text_with_package_context(source, "condition.gin");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("\"arith.cmpi\""), "{mlir}");
    assert!(mlir.contains("\"scf.if\""), "{mlir}");
}

#[test]
fn structural_and_condition_lowers_with_short_circuit_region() {
    let source = "\
choose: when 1 < 2 is True and 2 < 3 is True then 1 else 0
";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "and-condition.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.matches("\"scf.if\"").count() >= 2, "{mlir}");
    assert!(mlir.matches("\"arith.cmpi\"").count() >= 2, "{mlir}");
}

#[test]
fn structural_or_condition_lowers_with_short_circuit_region() {
    let source = "\
choose: when 2 < 1 is True or 2 < 3 is True then 1 else 0
";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "or-condition.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.matches("\"scf.if\"").count() >= 2, "{mlir}");
    assert!(mlir.matches("\"arith.cmpi\"").count() >= 2, "{mlir}");
}

#[test]
fn structural_not_condition_lowers_as_i1_negation() {
    let source = "\
choose: when not (2 < 1 is True) then 1 else 0
";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "not-condition.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("arith.xori"), "{mlir}");
}

#[test]
fn integer_literal_pattern_compares_the_stored_integer() {
    let source = "Word is in 0 to 255\nchoose(value Word) Word: when value is 0 then 1 else 2\n";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "integer-pattern.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("arith.cmpi"), "{mlir}");
    assert!(mlir.contains("scf.if"), "{mlir}");
}

#[test]
fn comparison_result_pattern_uses_its_anonymous_alternatives() {
    let source =
        "choose(value is in 0 to 255): when value < 3 is\n    True then 7\n    False then 9\n";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "result-pattern.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("arith.cmpi"), "{mlir}");
    assert!(mlir.contains("scf.if"), "{mlir}");
}

#[test]
fn integer_patterns_and_signed_returns_preserve_values_at_runtime() {
    for (source, expected) in [
        (
            "Word is in 0 to 255\nchoose(value Word) Word: when value is in -1 to 1 then 1 else 0\nmain: choose(255)\n",
            0,
        ),
        (
            "Word is in 0 to 255\nchoose(value Word) Word: when value is 256 then 1 else 0\nmain: choose(0)\n",
            0,
        ),
        ("main: -1\n", -1),
        (
            "Outcome is in 0 to 255\nchoose(a is in -9223372036854775808 to 9223372036854775807, b is in -9223372036854775808 to 9223372036854775807) Outcome: when a > b is True then 1 else 0\nmain: choose(0, -10)\n",
            1,
        ),
        (
            "Outcome is in 0 to 255\nchoose(a is in -9223372036854775808 to 9223372036854775807, b is in -9223372036854775808 to 9223372036854775807) Outcome: when a <= b is True then 1 else 0\nmain: choose(0, -10)\n",
            0,
        ),
        (
            "Outcome is in 0 to 255\nchoose(a is in 0 to 18446744073709551615, b is in 0 to 18446744073709551615) Outcome: when a > b is True then 1 else 0\nmain: choose(18446744073709551615, 0)\n",
            1,
        ),
        (
            "Outcome is in 0 to 255\nchoose(a is in 0 to 18446744073709551615, b is in 0 to 18446744073709551615) Outcome: when a <= b is True then 1 else 0\nmain: choose(18446744073709551615, 0)\n",
            0,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: next\nset(value mut Word):\n    value:: replace(deref value, 9)\nreturn\nguard(value mut Word, condition Word):\n    if condition is 1\n        set(value)\n    return\nreturn\nmain:\n    value Word: 0\n    guard(mut value, 1)\nreturn value\n",
            9,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: next\nset(value mut Word):\n    value:: replace(deref value, 9)\nreturn\nguard(value mut Word, condition Word):\n    if condition is 1\n        set(value)\n    return\nreturn\nmain:\n    value Word: 0\n    guard(mut value, 0)\nreturn value\n",
            0,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: next\nset(value mut Word):\n    value:: replace(deref value, 9)\nreturn\nmain:\n    value Word: 0\n    running Word: 1\n    while running is 1\n        set(mut value)\n        running:: replace(running, 0)\n    loop\nreturn value\n",
            9,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: next\nset(value mut Word):\n    value:: replace(deref value, 9)\nreturn\nmain:\n    value Word: 0\n    while value is 0\n        set(mut value)\n        value:: replace(value, 9)\n    loop\nreturn value\n",
            9,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: next\nset(value mut Word):\n    value:: replace(deref value, 9)\nreturn\nmain:\n    value Word: 0\n    for index in 0 to 3\n        set(mut value)\n    loop\nreturn value\n",
            9,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: next\nset(value mut Word):\n    value:: replace(deref value, 9)\nreturn\nchoose(condition Word, value mut Word):\n    when condition is 1 then set(value) else set(value)\nreturn\nmain:\n    value Word: 0\n    choose(1, mut value)\nreturn value\n",
            9,
        ),
    ] {
        let platform = if cfg!(target_os = "macos") {
            "apple-darwin"
        } else if cfg!(target_os = "windows") {
            "pc-windows-msvc"
        } else {
            "unknown-linux-gnu"
        };
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&format!("{}-{platform}", std::env::consts::ARCH))
                .expect("host target"),
        );
        let (text, diagnostics) = support::codegen_assembly_to_mlir_text_for_target(
            source,
            "pattern-runtime.gin",
            target,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let context = codegen::NativeCompiler::create_context();
        let module = melior::ir::Module::parse(&context, &text).expect("valid MLIR");
        let engine = codegen::NativeEngine::new(module, "main").expect("native engine");
        assert_eq!(
            engine.invoke_i64().expect("native result"),
            expected,
            "{source}\n{text}"
        );
    }
}
