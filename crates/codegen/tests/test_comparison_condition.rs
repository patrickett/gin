mod support;

#[test]
fn comparison_operators_lower_signed_and_unsigned_predicates() {
    let source = r#"Unsigned is in 0 to 255
Signed is in -128 to 127

unsigned_less_bits(a Unsigned, b Unsigned) False or True:
    left is in 0 to 255: a
    right is in 0 to 255: b
    return when left < right then True else False
signed_greater_equal_bits(a Signed, b Signed) False or True:
    left is in -128 to 127: a
    right is in -128 to 127: b
    return when left >= right then True else False

renamed_unsigned_comparison(a Unsigned, b Unsigned) False or True: return unsigned_less_bits(a, b)
renamed_signed_comparison(a Signed, b Signed) False or True: return signed_greater_equal_bits(a, b)

use_unsigned(a Unsigned, b Unsigned) False or True: return when a < b then True else False
use_signed(a Signed, b Signed) False or True: return when a >= b then True else False
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
    let source = "Flag is Enabled or Disabled\nchoose(value Flag) Flag: return when value is Flag.Enabled then Flag.Enabled else Flag.Disabled\n";
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text_with_package_context(source, "condition.gin");

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("\"arith.cmpi\""), "{mlir}");
    assert!(mlir.contains("\"scf.if\""), "{mlir}");
}

#[test]
fn structural_and_condition_lowers_with_short_circuit_region() {
    let source = "\
choose: when 1 < 2 and 2 < 3 then 1 else 0
";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "and-condition.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.matches("\"scf.if\"").count() >= 2, "{mlir}");
    assert!(mlir.matches("\"arith.cmpi\"").count() >= 2, "{mlir}");
}

#[test]
fn structural_or_condition_lowers_with_short_circuit_region() {
    let source = "\
choose: when 2 < 1 or 2 < 3 then 1 else 0
";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "or-condition.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.matches("\"scf.if\"").count() >= 2, "{mlir}");
    assert!(mlir.matches("\"arith.cmpi\"").count() >= 2, "{mlir}");
}

#[test]
fn structural_not_condition_lowers_as_i1_negation() {
    let source = "\
choose: when not (2 < 1) then 1 else 0
";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "not-condition.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("arith.xori"), "{mlir}");
}

#[test]
fn integer_literal_pattern_compares_the_stored_integer() {
    let source =
        "Word is in 0 to 255\nchoose(value Word) Word: return when value is 0 then 1 else 2\n";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "integer-pattern.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("arith.cmpi"), "{mlir}");
    assert!(mlir.contains("scf.if"), "{mlir}");
}

#[test]
fn stored_proposition_drives_runtime_branching() {
    let source = "choose(value is in 0 to 255):\n    small: value < 3\n    return when small then 7 else 9\n";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "result-pattern.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("arith.cmpi"), "{mlir}");
    assert!(mlir.contains("scf.if"), "{mlir}");
}

#[test]
#[allow(unsafe_code)]
fn integer_patterns_and_signed_returns_preserve_values_at_runtime() {
    for (source, expected) in [
        (
            "choose(value is in 0 to 255):\n    small: value < 16\n    return when small then 1 else 0\nmain: choose(7)\n",
            1,
        ),
        (
            "choose(value is in 0 to 255):\n    small: value < 16\n    return when small then 1 else 0\nmain: choose(200)\n",
            0,
        ),
        (
            "less(left ref is in 0 to 255, right is in 0 to 255) is left < right: return left < right\nchoose(value is in 0 to 255):\n    small: less(value, 16)\n    return when small then 1 else 0\nmain: choose(7)\n",
            1,
        ),
        (
            "consume(value is in 0 to 255, check is value < 16): return when check then 1 else 0\nchoose(value is in 0 to 255):\n    small: value < 16\n    return consume(value, small)\nmain: choose(200)\n",
            0,
        ),
        ("main: when 18446744073709551615 > -1 then 1 else 0\n", 1),
        ("main: when -1 < 18446744073709551615 then 1 else 0\n", 1),
        (
            "main: when 340282366920938463463374607431768211455 > -1 then 1 else 0\n",
            1,
        ),
        ("main: when 255 = -1 then 1 else 0\n", 0),
        ("main: when 255 in 0 to 255 then 1\n", 1),
        (
            "choose(value is in -128 to 255): return when value in 0 to 255 then 1 else 0\nmain: choose(-1)\n",
            0,
        ),
        (
            "Word is in 0 to 255\nchoose(value ref Word): return when value < 256 then 1 else 0\nmain:\n    value Word: 255\n    return choose(value)\n",
            1,
        ),
        (
            "Word is in 0 to 255\nchoose(value Word) Word: return when value is in -1 to 1 then 1 else 0\nmain:\n    value Word: 255\n    return choose(value)\n",
            0,
        ),
        (
            "Word is in 0 to 255\nchoose(value Word) Word: return when value is 256 then 1 else 0\nmain: choose(0)\n",
            0,
        ),
        ("main: -1\n", -1),
        (
            "Outcome is in 0 to 255\nchoose(a is in -9223372036854775808 to 9223372036854775807, b is in -9223372036854775808 to 9223372036854775807) Outcome: return when a > b then 1 else 0\nmain: choose(0, -10)\n",
            1,
        ),
        (
            "Outcome is in 0 to 255\nchoose(a is in -9223372036854775808 to 9223372036854775807, b is in -9223372036854775808 to 9223372036854775807) Outcome: return when a <= b then 1 else 0\nmain: choose(0, -10)\n",
            0,
        ),
        (
            "Outcome is in 0 to 255\nchoose(a is in 0 to 18446744073709551615, b is in 0 to 18446744073709551615) Outcome: return when a > b then 1 else 0\nmain: choose(18446744073709551615, 0)\n",
            1,
        ),
        (
            "Outcome is in 0 to 255\nchoose(a is in 0 to 18446744073709551615, b is in 0 to 18446744073709551615) Outcome: return when a <= b then 1 else 0\nmain: choose(18446744073709551615, 0)\n",
            0,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: return next\nset(value mut Word):\n    value:: replace(deref value, 9)\n    return\nguard(value mut Word, condition Word):\n    when condition is 1 then\n        set(value)\n        return\n    return\nmain:\n    value Word: 0\n    guard(value, 1)\n    return value\n",
            9,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: return next\nset(value mut Word):\n    value:: replace(deref value, 9)\n    return\nguard(value mut Word, condition Word):\n    when condition is 1 then\n        set(value)\n        return\n    return\nmain:\n    value Word: 0\n    guard(value, 0)\n    return value\n",
            0,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: return next\nset(value mut Word):\n    value:: replace(deref value, 9)\n    return\nmain:\n    value Word: 0\n    running Word: 1\n    while running is 1 do\n        set(value)\n        running:: replace(running, 0)\n    return value\n",
            9,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: return next\nset(value mut Word):\n    value:: replace(deref value, 9)\n    return\nmain:\n    value Word: 0\n    while value is 0 do\n        set(value)\n        value:: replace(value, 9)\n    return value\n",
            9,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: return next\nset(value mut Word):\n    value:: replace(deref value, 9)\n    return\nmain:\n    value Word: 0\n    for index in 0 to 3\n        set(value)\n    return value\n",
            9,
        ),
        (
            "Word is in 0 to 255\nreplace(previous Word, next Word) Word: return next\nset(value mut Word):\n    value:: replace(deref value, 9)\n    return\nchoose(condition Word, value mut Word):\n    when condition is 1 then set(value) else set(value)\n    return\nmain:\n    value Word: 0\n    choose(1, value)\n    return value\n",
            9,
        ),
    ] {
        let source = source
            .replace("main:", "#entry\nmain:")
            .replace(
                "Word is in 0 to 255\n",
                test_fixtures::preludes::WORD_PRELUDE,
            )
            .replace(
                "Outcome is in 0 to 255\n",
                "Outcome is in 0 to 255\nOutcome has\n    finish(self):\n        return\n",
            );
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
            &source,
            "pattern-runtime.gin",
            target,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let context = codegen::NativeCompiler::create_context();
        let module = melior::ir::Module::parse(&context, &text).expect("valid MLIR");
        let engine = codegen::NativeEngine::new(module, "main").expect("native engine");
        assert_eq!(
            unsafe { engine.invoke_i64() }.expect("native result"),
            expected,
            "{source}\n{text}"
        );
    }
}
