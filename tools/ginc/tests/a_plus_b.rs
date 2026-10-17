mod common;
use common::compile_exe;
use std::io::Write;
use std::process::{Command, Stdio};

use ginc::cli::Profile;
use test_fixtures::TempPackage;

#[test]
fn a_plus_b_example_reads_adds_and_prints_integers() {
    let package = TempPackage::new("a_plus_b_compile");
    let input = package.write(
        "main.gin",
        &test_fixtures::with_runtime(
            r#"#entry
main:
    left is in RuntimeInt: runtime_number
    right is in RuntimeInt: runtime_number
    when left is in -2147483648 to 2147483647 and right is in -2147483648 to 2147483647 then
        runtime_print((left + right) as RuntimeInt)
        return 0
    return 1
"#,
        ),
    );
    let output = package.join("a_plus_b");
    let result = compile_exe(
        input.parent().unwrap().to_path_buf(),
        &output.clone(),
        Profile::Debug,
    );
    assert!(result.is_success(), "A+B compilation failed: {result:?}");
    assert!(
        !result.has_typecheck_diagnostics(),
        "A+B typecheck failed: {result:?}"
    );

    for (input, expected, status) in [
        ("20 22\n", "42\n", 0),
        ("2147483647 2147483647\n", "4294967294\n", 0),
        ("-2147483648 -2147483648\n", "-4294967296\n", 0),
        ("2147483648 1\n", "", 1),
        ("1 -2147483649\n", "", 1),
    ] {
        let mut process = Command::new(&output)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("run compiled A+B example");
        process
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = process.wait_with_output().expect("wait for A+B example");
        assert_eq!(output.status.code(), Some(status), "{input}: {output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{input}");
    }
}
