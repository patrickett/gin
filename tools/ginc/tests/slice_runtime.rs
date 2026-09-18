use ginc::cli::Args;
use ginc::compile::GinCompiler;
use std::process::Command;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn imported_generic_slice_constructs_and_preserves_literal_byte_length() {
    let package = TempPackage::new("imported_slice");
    package.write_flask("byte_views");
    package.write(
        "views/views.gin",
        r#"Byte is in 0 to 255
Length is in 0 to 18446744073709551615
Slice(element) has ptr @element, len Length
"#,
    );
    let input = package.write(
        "main.gin",
        r#"use 'views' Slice, Byte, Length
rebuild(pointer @Byte, length Length) Slice(Byte): Slice(ptr: pointer, len: length)
static_bytes @Byte: (@'a\0b') as @Byte
main:
    original Slice(Byte): Slice(ptr: (@'é🦀') as @Byte, len: 6)
    rebuilt: rebuild(original.ptr, original.len)
    pointer: static_bytes
    zero is in 0 to 255: pointer.(1)
    tail is in 0 to 255: pointer.(2)
    empty := ''
    empty_length is in 0 to 255: #size(empty)
return when zero = 0 is True and tail = 98 is True and empty_length = 0 is True then rebuilt.len else 0
"#,
    );
    let executable = package.join("program");
    let mut args = Args {
        input,
        output: Some(executable.clone()),
        target: Some(host_target_triple()),
        no_cache: true,
        gin_home: Some(flask::GinHome::new(package.join("home"))),
        ..Default::default()
    };
    let result = GinCompiler::compile(&mut args);
    assert!(result.is_success(), "{result:?}");
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    let output = Command::new(executable).output().unwrap();
    assert_eq!(output.status.code(), Some(6), "{output:?}");
}
