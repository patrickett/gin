mod common;
use common::compile_exe;
use ginc::cli::Profile;
use std::process::Command;
use test_fixtures::TempPackage;

mod integer_cases;
mod integer_comparisons;
mod integer_conversions;
mod integer_returns;
mod result_families;

fn try_run_program(
    source: &str,
    name: &str,
    profile: Profile,
) -> Result<std::process::Output, String> {
    let package = TempPackage::new(name);
    let input = package.write("main.gin", source);
    let executable = package.join("program");
    let result = compile_exe(input, &executable.clone(), profile);
    if !result.is_success() || result.has_typecheck_diagnostics() {
        return Err(format!("{profile:?}: {result:?}"));
    }
    Command::new(executable)
        .output()
        .map_err(|error| error.to_string())
}

fn assert_program(source: &str, name: &str, profile: Profile) {
    let output = try_run_program(source, name, profile).unwrap();
    assert_eq!(output.status.code(), Some(0), "{profile:?}: {output:?}");
}

#[test]
fn small_negative_i128_literals_preserve_numeric_order_at_runtime() {
    let source = "compare(left is in 0 to 255, right is in -170141183460469231731687303715884105728 to 170141183460469231731687303715884105727) is in 0 to 1: return when left < right then 1 else 0\n#entry\nmain: compare(0, -1)\n";
    for profile in [Profile::Debug, Profile::Release] {
        assert_program(source, "negative_i128_literal", profile);
    }
}

#[test]
fn mixed_signed_and_unsigned_comparisons_preserve_numeric_order() {
    let unsigned = "0 to 255";
    let signed = "-128 to 127";
    let cases = [
        (unsigned, signed, "0", "-1", ">", 1),
        (unsigned, signed, "255", "-1", "=", 0),
        (unsigned, signed, "0", "-1", "<", 0),
        (unsigned, signed, "0", "-1", ">=", 1),
        (unsigned, signed, "0", "-1", "<=", 0),
        (signed, unsigned, "-1", "255", "<", 1),
        (signed, unsigned, "-1", "255", "=", 0),
        (signed, unsigned, "127", "255", ">", 0),
        (unsigned, signed, "127", "127", "=", 1),
        (
            "0 to 18446744073709551615",
            "-9223372036854775808 to 9223372036854775807",
            "18446744073709551615",
            "-1",
            ">",
            1,
        ),
        (
            "-9223372036854775808 to 9223372036854775807",
            "0 to 18446744073709551615",
            "-9223372036854775808",
            "18446744073709551615",
            "<",
            1,
        ),
        (
            "0 to 340282366920938463463374607431768211455",
            "-170141183460469231731687303715884105728 to 170141183460469231731687303715884105727",
            "340282366920938463463374607431768211455",
            "-1",
            ">",
            1,
        ),
        (
            "0 to 340282366920938463463374607431768211455",
            "-170141183460469231731687303715884105728 to 170141183460469231731687303715884105727",
            "largest_unsigned",
            "-1",
            ">",
            1,
        ),
    ];
    let mut source = "largest_unsigned := 340282366920938463463374607431768211455\n".to_string();
    let mut checks = Vec::new();
    for (index, (left_type, right_type, left, right, operator, expected)) in
        cases.into_iter().enumerate()
    {
        source.push_str(&format!(
            "compare_{index}(left is in {left_type}, right is in {right_type}) is in 0 to 1: return when left {operator} right then 1 else 0\n"
        ));
        checks.push(format!("compare_{index}({left}, {right}) = {expected}"));
    }
    let result =
        checks
            .iter()
            .enumerate()
            .rev()
            .fold("0".to_string(), |continuation, (index, check)| {
                format!("when {check} then ({continuation}) else {}", index + 1)
            });
    source.push_str(&format!("#entry\nmain: {result}\n"));
    for profile in [Profile::Debug, Profile::Release] {
        assert_program(&source, "mixed_integer_comparisons", profile);
    }
}

#[test]
fn short_circuit_conditions_do_not_execute_division_by_zero() {
    let source = "guarded_and(numerator is in -128 to 127, divisor is in -128 to 127) is in 0 to 1: return when not (divisor = 0) and numerator / divisor > 0 then 1 else 0\nguarded_or(numerator is in -128 to 127, divisor is in -128 to 127) is in 0 to 1: return when divisor = 0 or numerator / divisor > 0 then 1 else 0\n#entry\nmain: when guarded_and(1, 0) = 0 and guarded_or(1, 0) = 1 and guarded_and(4, 2) = 1 and guarded_or(-4, 2) = 0 then 0 else 1\n";
    for profile in [Profile::Debug, Profile::Release] {
        assert_program(source, "short_circuit_division", profile);
    }
}

#[test]
fn borrowed_integer_proofs_survive_generic_forwarding() {
    let source = r#"Byte is in 0 to 255
Byte has finish(self): return
less(left ref is in 0 to 511, right is in 0 to 511) True thus left < right or False thus left >= right:
    when left < right then
        return True
    return False
identity(value x): return value
narrow(value is in 0 to 511) Byte:
    when identity(less(value, 256)) is True then
        return value as Byte
    return 0
raw_byte(value Byte) is in 0 to 255: return value
#entry
main:
    for value in 0 to 511
        expected: when value < 256 then value + 0 else 0
        actual: raw_byte(narrow(value))
        when not (actual = expected) then
            return 1
    return 0
"#;
    for profile in [Profile::Debug, Profile::Release] {
        assert_program(source, "borrowed_integer_proof", profile);
    }
}
