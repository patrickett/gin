use std::process::Command;

use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn hello_world_example_writes_expected_stdout() {
    let package = TempPackage::new("hello_world_compile");
    let input = package.write(
        "main.gin",
        &test_fixtures::with_runtime(
            r#"SignedInt is in -9223372036854775808 to 9223372036854775807
SignedInt has
    finish(self):
        return

runtime_print_int(value SignedInt) SignedInt: return runtime_print(value as RuntimeInt) as SignedInt
#entry
main:
    runtime_print_int(42)
    return 0
"#,
        ),
    );
    let output = package.join("hello_world");
    let mut args = Args {
        input: input.parent().unwrap().to_path_buf(),
        output: Some(output.clone()),
        emit: Emit::Exe,
        profile: Profile::Debug,
        target: Some(host_target_triple()),
        ..Default::default()
    };

    let result = GinCompiler::compile(&mut args);
    assert!(
        result.is_success(),
        "hello world compilation failed: {result:?}"
    );

    let process = Command::new(&output)
        .output()
        .expect("run compiled hello world example");
    assert!(
        process.status.success(),
        "hello world exited unsuccessfully"
    );
    assert_eq!(String::from_utf8_lossy(&process.stdout), "42\n");
}
