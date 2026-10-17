mod common;
use common::compile_and_run_exe;
use ginc::cli::Profile;
use test_fixtures::TempPackage;

#[test]
fn checked_named_integer_conversion_preserves_runtime_width_and_boundaries() {
    let package = TempPackage::new("numeric_narrowing");
    package.write(
        "main.gin",
        &format!("\nSigned is in -9223372036854775808 to 9223372036854775807\nSigned has\n    finish(self):\n        return\n{}narrow(value Signed) Byte: return when value is in 0 to 255 then value as Byte else 0\ncheck(value Signed, expected Byte) is in 0 to 1:\n    actual is in 0 to 255: narrow(value)\n    expected_bits is in 0 to 255: expected\n    return when actual = expected_bits then 0 else 1\n#entry\nmain:\n    negative is in 0 to 255: check(-1, 0)\n    zero is in 0 to 255: check(0, 0)\n    high_bit is in 0 to 255: check(128, 128)\n    maximum is in 0 to 255: check(255, 255)\n    outside is in 0 to 255: check(256, 0)\n    signed_min is in 0 to 255: check(-9223372036854775808, 0)\n    signed_max is in 0 to 255: check(9223372036854775807, 0)\n    return negative + zero + high_bit + maximum + outside + signed_min + signed_max\n", test_fixtures::preludes::BYTE_PRELUDE),
    );
    let output = compile_and_run_exe(
        package.join("main.gin").parent().unwrap().to_path_buf(),
        &package.join("narrowing"),
        Profile::Debug,
        "",
        "run numeric narrowing",
    );
    assert!(output.status.success());
}
