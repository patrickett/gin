use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use std::io::Write;
use std::process::{Command, Stdio};
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn compiled_return_row_forwarding_preserves_runtime_named_projections() {
    let (target, input, sum, carry, template) = match host_target_triple().as_str() {
        "x86_64-unknown-linux-gnu" => (
            "x86_64-unknown-linux-gnu",
            "rdi",
            "rax",
            "rdx",
            "mov rax, rdi\\\\nadd rax, 1\\\\nmov rdx, rdi\\\\nadd rdx, 2",
        ),
        "aarch64-apple-darwin" => (
            "aarch64-apple-darwin",
            "x0",
            "x1",
            "x2",
            "mov x1, x0\\\\nadd x1, x1, #1\\\\nmov x2, x0\\\\nadd x2, x2, #2",
        ),
        target => panic!("no assembly fixture for host target {target}"),
    };
    let package = TempPackage::new("return_row_runtime");
    let source = format!(
        r#"Word is in 0 to 18446744073709551615

runtime_read_int() Word: runtime_number() as Word
runtime_print_int(value Word) Word: runtime_print(value as RuntimeInt) as Word
pair(value Word) sum Word, carry Word: #asm('gin-asm 1\ntarget "{target}"\ndialect att\ninput "value" fixed "{input}"\noutput "sum" fixed "{sum}"\noutput "carry" fixed "{carry}"\ntemplate "{template}"', value: value)
forward(value Word) sum Word, carry Word: return pair(value)

main:
    value := runtime_read_int
    result := forward(value)
    runtime_print_int(result.sum)
    runtime_print_int(result.carry)
return 0
"#
    );
    let input_file = package.write("main.gin", &test_fixtures::with_runtime(&source));
    let output = package.join("return_row_runtime");
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
        "return-row compilation failed: {result:?}"
    );
    let mut process = Command::new(&output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run compiled assembly example");
    process.stdin.take().unwrap().write_all(b"20\n").unwrap();
    let output = process.wait_with_output().expect("wait for example");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "21\n22\n");
}

#[test]
fn compiled_nonzero_tied_input_preserves_forwarded_named_projections() {
    let (target, input, sum, carry, template) = match host_target_triple().as_str() {
        "x86_64-unknown-linux-gnu" => (
            "x86_64-unknown-linux-gnu",
            "rdx",
            "rax",
            "rdx",
            "lea 1(%rdx), %rax",
        ),
        "aarch64-apple-darwin" => ("aarch64-apple-darwin", "x2", "x1", "x2", "add x1, x2, #1"),
        target => panic!("no assembly fixture for host target {target}"),
    };
    let package = TempPackage::new("nonzero_tied_return_row_runtime");
    let source = format!(
        r#"Word is in 0 to 18446744073709551615

runtime_read_int() Word: runtime_number() as Word
runtime_print_int(value Word) Word: runtime_print(value as RuntimeInt) as Word
pair(value Word) sum Word, carry Word: #asm('gin-asm 1\ntarget \"{target}\"\ndialect att\ninput \"value\" tied \"{input}\"\noutput \"sum\" fixed \"{sum}\"\noutput \"carry\" fixed \"{carry}\"\ntemplate \"{template}\"', value: value)
forward(value Word) sum Word, carry Word: return pair(value)

main:
    value := runtime_read_int
    stored := forward(value)
    runtime_print_int(stored.sum)
    runtime_print_int(stored.carry)
return 0
"#
    );
    let input_file = package.write("main.gin", &test_fixtures::with_runtime(&source));
    let output = package.join("nonzero_tied_return_row_runtime");
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
    assert!(result.is_success(), "compilation failed: {result:?}");
    let mut process = Command::new(&output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run compiled tied assembly example");
    process.stdin.take().unwrap().write_all(b"20\n").unwrap();
    let output = process.wait_with_output().expect("wait for example");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "21\n20\n");
}
