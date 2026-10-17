use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use std::io::Write;
use std::process::{Command, Stdio};
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn compiled_return_row_forwarding_preserves_runtime_named_projections() {
    let target = host_target_triple();
    let machine = pair_encoding(&target, false);
    let package = TempPackage::new("return_row_runtime");
    let source = format!(
        r#"Word is in 0 to 18446744073709551615
Word has
    finish(self):
        return

runtime_read_int Word: runtime_word_bits(runtime_number) as Word
runtime_print_int(value Word) Word: return runtime_word_bits(runtime_print(runtime_signed_bits(value as RuntimeWord))) as Word
{machine}
forward(value Word) sum Word, carry Word: return pair(value)

#entry
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
        gin_home: Some(flask::GinHome::new(package.join("home"))),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(
        result.is_success(),
        "return-row compilation failed: {result:?}"
    );
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
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
    let target = host_target_triple();
    let machine = pair_encoding(&target, true);
    let package = TempPackage::new("nonzero_tied_return_row_runtime");
    let source = format!(
        r#"Word is in 0 to 18446744073709551615
Word has
    finish(self):
        return

runtime_read_int Word: runtime_word_bits(runtime_number) as Word
runtime_print_int(value Word) Word: return runtime_word_bits(runtime_print(runtime_signed_bits(value as RuntimeWord))) as Word
{machine}
forward(value Word) sum Word, carry Word: return pair(value)

#entry
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
        gin_home: Some(flask::GinHome::new(package.join("home"))),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(result.is_success(), "compilation failed: {result:?}");
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
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

fn pair_encoding(target: &str, tied: bool) -> &'static str {
    match (target, tied) {
        ("x86_64-unknown-linux-gnu", false) => {
            r#"PairInputs has
    value Word
    finish(self):
        return
#phantom(input, output)
PairSignature(input, output) has
    version Word
    finish(self):
        return
pair_signature PairSignature(PairInputs, [sum Word, carry Word]): PairSignature(version: 1)
pair(value Word) sum Word, carry Word:
    code := [signature: pair_signature, target: #target.triple, bytes: Encoding0.encode, inputs: [value: [architecture: "x86_64", bank: "general", index: 7, width: 64]], outputs: [sum: [architecture: "x86_64", bank: "general", index: 0, width: 64], carry: [architecture: "x86_64", bank: "general", index: 2, width: 64]], clobbers: [], reads: [], writes: []]
    return #emit(code, PairInputs(value: value))


Encoding0 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 14): return [72 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 248 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 192 as RuntimeEncodingByte, 1 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 137 as RuntimeEncodingByte, 250 as RuntimeEncodingByte, 72 as RuntimeEncodingByte, 131 as RuntimeEncodingByte, 194 as RuntimeEncodingByte, 2 as RuntimeEncodingByte]

"#
        }
        ("aarch64-apple-darwin", false) => {
            r#"PairInputs has
    value Word
    finish(self):
        return
#phantom(input, output)
PairSignature(input, output) has
    version Word
    finish(self):
        return
pair_signature PairSignature(PairInputs, [sum Word, carry Word]): PairSignature(version: 1)
pair(value Word) sum Word, carry Word:
    code := [signature: pair_signature, target: #target.triple, bytes: Encoding0.encode, inputs: [value: [architecture: "aarch64", bank: "general", index: 0, width: 64]], outputs: [sum: [architecture: "aarch64", bank: "general", index: 1, width: 64], carry: [architecture: "aarch64", bank: "general", index: 2, width: 64]], clobbers: [], reads: [], writes: []]
    return #emit(code, PairInputs(value: value))


Encoding0 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 16): return [225 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 33 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 145 as RuntimeEncodingByte, 226 as RuntimeEncodingByte, 3 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 170 as RuntimeEncodingByte, 66 as RuntimeEncodingByte, 8 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 145 as RuntimeEncodingByte]

"#
        }
        ("x86_64-unknown-linux-gnu", true) => {
            r#"PairInputs has
    value Word
    finish(self):
        return
#phantom(input, output)
PairSignature(input, output) has
    version Word
    finish(self):
        return
pair_signature PairSignature(PairInputs, [sum Word, carry Word]): PairSignature(version: 1)
pair(value Word) sum Word, carry Word:
    code := [signature: pair_signature, target: #target.triple, bytes: Encoding0.encode, inputs: [value: [architecture: "x86_64", bank: "general", index: 2, width: 64]], outputs: [sum: [architecture: "x86_64", bank: "general", index: 0, width: 64], carry: [architecture: "x86_64", bank: "general", index: 2, width: 64]], clobbers: [], reads: [], writes: []]
    return #emit(code, PairInputs(value: value))


Encoding0 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 4): return [72 as RuntimeEncodingByte, 141 as RuntimeEncodingByte, 66 as RuntimeEncodingByte, 1 as RuntimeEncodingByte]

"#
        }
        ("aarch64-apple-darwin", true) => {
            r#"PairInputs has
    value Word
    finish(self):
        return
#phantom(input, output)
PairSignature(input, output) has
    version Word
    finish(self):
        return
pair_signature PairSignature(PairInputs, [sum Word, carry Word]): PairSignature(version: 1)
pair(value Word) sum Word, carry Word:
    code := [signature: pair_signature, target: #target.triple, bytes: Encoding0.encode, inputs: [value: [architecture: "aarch64", bank: "general", index: 2, width: 64]], outputs: [sum: [architecture: "aarch64", bank: "general", index: 1, width: 64], carry: [architecture: "aarch64", bank: "general", index: 2, width: 64]], clobbers: [], reads: [], writes: []]
    return #emit(code, PairInputs(value: value))


Encoding0 has
    encode() RuntimeEncodingBytes(RuntimeEncodingByte, 4): return [65 as RuntimeEncodingByte, 4 as RuntimeEncodingByte, 0 as RuntimeEncodingByte, 145 as RuntimeEncodingByte]

"#
        }
        _ => panic!("no machine fixture for target {target}"),
    }
}
