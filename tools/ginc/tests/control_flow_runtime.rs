mod common;
use common::{compile_exe, compile_exe_checked};
use ginc::cli::Profile;
use std::io::Write;
use std::process::{Command, Stdio};
use test_fixtures::TempPackage;

#[test]
fn compiled_range_loop_includes_the_endpoint_and_carries_rebind_value() {
    let package = TempPackage::new("control_flow_runtime");
    let source = r#"Word is in 0 to 18446744073709551615
Word has
    finish(self):
        return

add_bits(a Word, b Word) Word:
    left is in 0 to 18446744073709551615: a
    right is in 0 to 18446744073709551615: b
    return ((left + right) & 18446744073709551615) as Word
#infix('+')
add(a Word, b Word) Word: return add_bits(a, b)

runtime_read_int() Word: return runtime_word_bits(runtime_number) as Word
runtime_print_int(value Word) Word: return runtime_word_bits(runtime_print(runtime_signed_bits(value as RuntimeWord))) as Word
#entry
main() Word:
    count := runtime_read_int
    total Word: 0
    for index in 0 to count
        total :: add(total, 1)
    runtime_print_int(total)
    return 0
"#;
    let input_file = package.write("main.gin", &test_fixtures::with_runtime(source));
    let output = package.join("control_flow_runtime");
    let result = compile_exe(
        input_file.parent().unwrap().to_path_buf(),
        &output.clone(),
        Profile::Debug,
    );
    assert!(
        result.is_success(),
        "control-flow compilation failed: {result:?}"
    );
    assert!(
        !result.has_typecheck_diagnostics(),
        "control-flow typecheck failed: {result:?}"
    );

    for (input, expected) in [(b"0\n".as_slice(), "1\n"), (b"3\n".as_slice(), "4\n")] {
        let mut process = Command::new(&output)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("run compiled control-flow example");
        process.stdin.take().unwrap().write_all(input).unwrap();
        let output = process.wait_with_output().expect("wait for example");
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    }
}

#[test]
fn countdown_prints_each_value_then_blast_off() {
    let source = r#"#entry
main:
    for num in 10 to 0
        runtime_print(num as RuntimeInt)
    runtime_write(1, (@'blast off\n') as @RuntimeByte, 10)
    return 0
"#;
    let zero_branch = r#"#entry
main:
    for num in 10 to 0
        when num is 0
        then runtime_write(1, (@'blast off\n') as @RuntimeByte, 10)
        else runtime_print(num as RuntimeInt)
    return 0
"#;
    for (source, expected) in [
        (source, "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\n0\nblast off\n"),
        (zero_branch, "10\n9\n8\n7\n6\n5\n4\n3\n2\n1\nblast off\n"),
    ] {
        for profile in [Profile::Debug, Profile::Release] {
            let package = TempPackage::new("countdown_runtime");
            let input = package.write("main.gin", &test_fixtures::with_runtime(source));
            let executable = package.join("countdown");
            compile_exe_checked(input, &executable, profile);
            let output = Command::new(executable).output().unwrap();
            assert!(output.status.success(), "{profile:?}: {output:?}");
            assert_eq!(output.stdout, expected.as_bytes());
        }
    }
}

#[test]
fn input_bound_selects_signed_direction_at_runtime() {
    let source = r#"#entry
main:
    start is in RuntimeInt: runtime_number
    for num in start to 0
        runtime_print(num as RuntimeInt)
    return 0
"#;
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("runtime_range_direction");
        let input = package.write("main.gin", &test_fixtures::with_runtime(source));
        let executable = package.join("range");
        compile_exe_checked(input, &executable, profile);
        for (input, expected) in [
            ("3\n", "3\n2\n1\n0\n"),
            ("-3\n", "-3\n-2\n-1\n0\n"),
            ("0\n", "0\n"),
        ] {
            let mut process = Command::new(&executable)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            process
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
            let output = process.wait_with_output().unwrap();
            assert!(output.status.success(), "{profile:?}: {output:?}");
            assert_eq!(output.stdout, expected.as_bytes(), "{profile:?}: {input}");
        }
    }
}
