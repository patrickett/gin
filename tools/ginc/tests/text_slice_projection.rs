use ginc::cli::{Args, Profile};
use ginc::compile::GinCompiler;
use std::io::Write;
use std::process::{Command, Stdio};
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn text_fields_and_pointer_integer_roundtrip_execute() {
    let source = r#"ViewByte is in 0 to 255
ViewLength is in 0 to 18446744073709551615
Slice(element) has ptr @element, len ViewLength
Word is in 0 to 18446744073709551615
Byte is in 0 to 255
runtime_read_int Word: runtime_word_bits(runtime_number) as Word
runtime_print_int(value Word) Word: runtime_word_bits(runtime_print(runtime_signed_bits(value as RuntimeWord))) as Word
pointer(bits Word) @Word: bits as @Word
bits(address @Word) Word: address as Word
text Slice(ViewByte) := Slice(ptr: (@'é🦀') as @ViewByte, len: 6)
length(text Slice(ViewByte)) Word: text.len as Word
first(text Slice(ViewByte)) Byte: deref (text.ptr as @Byte)
view(pointer @Byte, length Word) Slice(ViewByte): Slice(ptr: pointer as @ViewByte, len: length as ViewLength)
main:
    value: runtime_read_int
    encoded: bits(@value)
    address: pointer(encoded)
    runtime_print_int(deref address)
    runtime_print_int(length(text))
    rebuilt: view(text.ptr as @Byte, text.len as Word)
    runtime_print_int(first(rebuilt) as Word)
return 0
"#;
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("text_pointer_roundtrip");
        let cache = TempPackage::new("text_pointer_cache");
        let executable = package.join("program");
        let mut args = Args {
            input: package.write("main.gin", &test_fixtures::with_runtime(source)),
            output: Some(executable.clone()),
            target: Some(host_target_triple()),
            profile,
            no_cache: true,
            gin_home: Some(flask::GinHome::new(cache.path().to_path_buf())),
            ..Default::default()
        };
        let result = GinCompiler::compile(&mut args);
        assert!(result.is_success(), "{result:?}");
        assert!(!result.has_typecheck_diagnostics(), "{result:?}");
        let mut child = Command::new(&executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"42\n").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n6\n195\n");
    }
}

#[test]
fn package_reference_reads_preserve_the_parameter_reference_type() {
    let library = TempPackage::new("reference_word_library");
    library.write_flask("words");
    library.write(
        "words.gin",
        &test_fixtures::with_runtime(r#"Word is in 0 to 18446744073709551615
runtime_print_int(value Word) Word: runtime_word_bits(runtime_print(runtime_signed_bits(value as RuntimeWord))) as Word
Pair has first Word, second Word
Pair.new(first Word, second Word) Pair: Pair(first: first, second: second)
Pair.emit(self own):
    Pair(first: left, second: right) := self
    runtime_print_int(left)
    runtime_print_int(right)
return
emit_pair(first Word, second Word):
    pair: Pair.new(first, second)
    Pair.emit(pair)
return
"#),
    );
    let package = TempPackage::new("reference_word_consumer");
    package.write_flask_with_deps(
        "consumer",
        &format!(
            r#"{{"words":{{"path":"../{}"}}}}"#,
            library.path().file_name().unwrap().to_string_lossy()
        ),
    );
    let source = r#"use words Word, emit_pair
read(value ref Word) Word: deref value
runtime_read_int Word: runtime_word_bits(runtime_number) as Word
main:
    value: runtime_read_int
    copied: read(ref value)
    emit_pair(copied, 9)
return 0
"#;
    let executable = package.join("program");
    package.write("main.gin", &test_fixtures::with_runtime(source));
    let mut args = Args {
        input: package.path().to_path_buf(),
        output: Some(executable.clone()),
        target: Some(host_target_triple()),
        no_cache: true,
        dependencies: [("words".to_string(), library.path().to_path_buf())].into(),
        gin_home: Some(flask::GinHome::new(package.join("cache"))),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(result.is_success(), "{result:?}");
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"42\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n9\n");
}
