use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use std::process::Command;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn checked_named_integer_conversion_preserves_runtime_width_and_boundaries() {
    let package = TempPackage::new("numeric_narrowing");
    package.write(
        "main.gin",
        r#"
Signed is in -9223372036854775808 to 9223372036854775807
Byte is in 0 to 255
narrow(value Signed) Byte: when value is in 0 to 255 then value as Byte else 0
check(value Signed, expected Byte) Signed:
    actual is in 0 to 255: narrow(value)
    expected_bits is in 0 to 255: expected
return when actual = expected_bits is True then 0 else 1
main:
    negative is in 0 to 255: check(-1, 0)
    zero is in 0 to 255: check(0, 0)
    high_bit is in 0 to 255: check(128, 128)
    maximum is in 0 to 255: check(255, 255)
    outside is in 0 to 255: check(256, 0)
    signed_min is in 0 to 255: check(-9223372036854775808, 0)
    signed_max is in 0 to 255: check(9223372036854775807, 0)
return negative + zero + high_bit + maximum + outside + signed_min + signed_max
"#,
    );
    let output = package.join("narrowing");
    let mut args = Args {
        input: package.join("main.gin").parent().unwrap().to_path_buf(),
        output: Some(output.clone()),
        emit: Emit::Exe,
        profile: Profile::Debug,
        target: Some(host_target_triple()),
        no_cache: true,
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(result.is_success(), "{result:?}");
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    assert!(Command::new(output).status().unwrap().success());
}
