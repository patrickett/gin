mod common;
use common::compile_exe;
use std::io::Write;
use std::process::{Command, Stdio};

use ginc::cli::Profile;
use test_fixtures::TempPackage;

#[test]
fn echo_example_copies_stdin_to_stdout() {
    let package = TempPackage::new("echo_compile");
    let input = package.write(
        "main.gin",
        &test_fixtures::with_runtime(
            r#"SignedInt is in -2147483648 to 2147483647
SignedInt has
    finish(self):
        return

runtime_echo() SignedInt: return runtime_copy_input() as SignedInt
echo:
    runtime_echo
    return

#entry
main:
    echo
    return 0
"#,
        ),
    );
    let output = package.join("echo");
    let result = compile_exe(
        input.parent().unwrap().to_path_buf(),
        &output.clone(),
        Profile::Debug,
    );
    assert!(result.is_success(), "echo compilation failed: {result:?}");

    let mut process = Command::new(&output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run compiled echo example");
    process
        .stdin
        .take()
        .expect("child stdin")
        .write_all(b"Gin input\nwith two lines\n")
        .expect("write echo input");
    let output = process.wait_with_output().expect("wait for echo example");

    assert!(output.status.success(), "echo exited unsuccessfully");
    assert_eq!(output.stdout, b"Gin input\nwith two lines\n");
}
