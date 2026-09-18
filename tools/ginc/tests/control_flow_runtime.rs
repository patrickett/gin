use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use std::io::Write;
use std::process::{Command, Stdio};
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn compiled_range_loop_is_exclusive_and_carries_rebind_value() {
    let package = TempPackage::new("control_flow_runtime");
    let source = r#"Word is in 0 to 18446744073709551615

add_bits(a Word, b Word) Word:
    left is in 0 to 18446744073709551615: a
    right is in 0 to 18446744073709551615: b
return (left + right) as Word
#infix('+')
add(a Word, b Word) Word: add_bits(a, b)

runtime_read_int() Word: runtime_word_bits(runtime_number) as Word
runtime_print_int(value Word) Word: runtime_word_bits(runtime_print(runtime_signed_bits(value as RuntimeWord))) as Word
main() Word:
    count := runtime_read_int
    total Word: 0
    for index in 0 to count
        total :: add(total, 1)
    loop
    runtime_print_int(total)
return 0
"#;
    let input_file = package.write("main.gin", &test_fixtures::with_runtime(source));
    let output = package.join("control_flow_runtime");
    let mut args = Args {
        input: input_file.parent().unwrap().to_path_buf(),
        output: Some(output.clone()),
        emit: Emit::Exe,
        profile: Profile::Debug,
        target: Some(host_target_triple()),
        no_cache: true,
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(
        result.is_success(),
        "control-flow compilation failed: {result:?}"
    );
    assert!(
        !result.has_typecheck_diagnostics(),
        "control-flow typecheck failed: {result:?}"
    );

    for (input, expected) in [(b"0\n".as_slice(), "0\n"), (b"3\n".as_slice(), "3\n")] {
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
