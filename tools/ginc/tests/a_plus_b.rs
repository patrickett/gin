use std::io::Write;
use std::process::{Command, Stdio};

use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn a_plus_b_example_reads_adds_and_prints_integers() {
    let package = TempPackage::new("a_plus_b_compile");
    let input = package.write(
        "main.gin",
        &test_fixtures::with_runtime(
            r#"SignedInt is in -2147483648 to 2147483647

add_bits(a SignedInt, b SignedInt) SignedInt:
    left is in -2147483648 to 2147483647: a
    right is in -2147483648 to 2147483647: b
return (left + right) as SignedInt
#infix('+')
add(a SignedInt, b SignedInt) SignedInt: add_bits(a, b)

runtime_read_int() SignedInt: runtime_number() as SignedInt
runtime_print_int(value SignedInt) SignedInt: runtime_print(value as RuntimeInt) as SignedInt
main:
    left := runtime_read_int
    right := runtime_read_int
    runtime_print_int(left + right)
return 0
"#,
        ),
    );
    let output = package.join("a_plus_b");
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
    assert!(result.is_success(), "A+B compilation failed: {result:?}");

    let mut process = Command::new(&output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run compiled A+B example");
    process
        .stdin
        .take()
        .expect("child stdin")
        .write_all(b"20 22\n")
        .expect("write A+B input");
    let output = process.wait_with_output().expect("wait for A+B example");

    assert!(
        output.status.success(),
        "A+B exited unsuccessfully: status={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n");
}
