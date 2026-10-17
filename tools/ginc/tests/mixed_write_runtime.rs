mod common;
use common::compile_exe;
use std::process::Command;

use ginc::cli::Profile;
use test_fixtures::TempPackage;

#[test]
fn mixed_local_write_and_runtime_extern_compile_and_run() {
    let package = TempPackage::new("mixed_write_runtime_compile");
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
    runtime_print_int(7)
    return 0
"#,
        ),
    );
    let output = package.join("mixed_write_runtime");
    let result = compile_exe(
        input.parent().unwrap().to_path_buf(),
        &output.clone(),
        Profile::Debug,
    );
    assert!(result.is_success(), "mixed compilation failed: {result:?}");
    let output = Command::new(output).output().expect("run mixed example");
    assert!(output.status.success(), "mixed example failed");
    assert_eq!(output.stdout, b"7\n");
}
