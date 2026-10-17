mod common;
use common::{compile_exe_with_gin_home, require_success};
use ginc::cli::Profile;
use std::process::Command;
use test_fixtures::TempPackage;

#[test]
fn imported_generic_slice_constructs_and_preserves_literal_byte_length() {
    let package = TempPackage::new("imported_slice");
    package.write_flask("byte_views");
    package.write(
        "views/views.gin",
        &format!("{}Length is in 0 to 18446744073709551615\nLength has\n    finish(self):\n        return\nSlice(element) has\n    ptr @element, len Length\n    finish(self):\n        return\nrebuild(value Slice(Byte)) Slice(Byte): return Slice(ptr: value.ptr, len: value.len)\n", test_fixtures::preludes::BYTE_PRELUDE),
    );
    let input = package.write(
        "main.gin",
        r#"use 'views' Slice, Byte, Length, rebuild
static_bytes @Byte: (@'a\0b') as @Byte
#entry
main:
    original Slice(Byte): Slice(ptr: (@'é🦀') as @Byte, len: 6)
    rebuilt: rebuild(original)
    pointer: static_bytes
    zero is in 0 to 255: pointer.[1]
    tail is in 0 to 255: pointer.[2]
    empty := ''
    empty_length is in 0 to 255: #size(empty)
    length is in 0 to 18446744073709551615: rebuilt.len
    rebuilt.finish
    return when zero = 0 and tail = 98 and empty_length = 0 then length else 0
"#,
    );
    let executable = package.join("program");
    let result = compile_exe_with_gin_home(input, &executable, package.join("home"));
    require_success(&result, Profile::Debug);
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    let output = Command::new(executable).output().unwrap();
    assert_eq!(output.status.code(), Some(6), "{output:?}");
}
