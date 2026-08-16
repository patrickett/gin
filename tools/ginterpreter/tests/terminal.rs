use std::io::Write;
use std::process::{Command, Stdio};
use test_fixtures::TempPackage;

const NUMBERS: &str = r#"#default(IntegerLiteral)
Signed64 is in -9223372036854775808...9223372036854775807

#intrinsic(BitsAdd)
signed64_add_bits(eat lhs Signed64, eat rhs Signed64) Signed64 extern
#operator(Add)
signed64_add(lhs Signed64, rhs Signed64) Signed64: signed64_add_bits(eat lhs, eat rhs)
"#;

#[test]
fn standalone_terminal_evaluates_integer_addition() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("interpreter process");
    child
        .stdin
        .take()
        .expect("interpreter stdin")
        .write_all(b"2 + 2\n#q\n")
        .expect("terminal submissions");

    let output = child.wait_with_output().expect("interpreter output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "stderr:\n{stderr}");
    assert!(stdout.contains("4\n"), "stdout:\n{stdout}");
}

#[test]
fn terminal_recovers_after_diagnostic_and_executes_again() {
    let package = TempPackage::new("interpreter_terminal");
    package.write_flask("interpreter_terminal");
    package.write("numbers/defs.gin", NUMBERS);
    let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
        .arg(package.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("interpreter process");
    child
        .stdin
        .take()
        .expect("interpreter stdin")
        .write_all(
            b"use 'numbers'.(Signed64, signed64_add_bits, signed64_add)\n40 + 2\nmissing + 2\n40 + 2\n#q\n",
        )
        .expect("terminal submissions");

    let output = child.wait_with_output().expect("interpreter output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "stderr:\n{stderr}");
    assert_eq!(
        stdout.matches("42\n").count(),
        2,
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stderr.contains("type-unknown-symbol"), "stderr:\n{stderr}");
}
