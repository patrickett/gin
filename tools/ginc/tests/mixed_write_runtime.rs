use std::process::Command;

use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn mixed_local_write_and_runtime_extern_compile_and_run() {
    let package = TempPackage::new("mixed_write_runtime_compile");
    let input = package.write(
        "main.gin",
        &test_fixtures::with_runtime(
            r#"SignedInt is in -2147483648 to 2147483647

runtime_print_int(value SignedInt) SignedInt: runtime_print(value as RuntimeInt) as SignedInt
main:
    runtime_print_int(7)
return 0
"#,
        ),
    );
    let output = package.join("mixed_write_runtime");
    let mut args = Args {
        input: input.parent().unwrap().to_path_buf(),
        output: Some(output.clone()),
        emit: Emit::Exe,
        profile: Profile::Debug,
        target: Some(host_target_triple()),
        no_cache: true,
        ..Default::default()
    };

    let result = GinCompiler::compile(&mut args);
    assert!(result.is_success(), "mixed compilation failed: {result:?}");
    let output = Command::new(output).output().expect("run mixed example");
    assert!(output.status.success(), "mixed example failed");
    assert_eq!(output.stdout, b"7\n");
}
