mod common;
use common::{compile_exe_with_gin_home, require_success};
use ginc::cli::Profile;
use std::process::Command;
use test_fixtures::TempPackage;

#[test]
fn imported_functions_infer_array_lengths_and_execute_specialized_bodies() {
    let package = TempPackage::new("inferred_array_length");
    package.write_flask("array_consumer");
    package.write(
        "arrays/arrays.gin",
        &format!("{}Length is in 0 to 18446744073709551615\nLength has\n    finish(self):\n        return\nsize(bytes [in Byte; count]) Length: return #size(bytes) as Length\nforward(bytes [in Byte; count]) Length: return size(bytes)\nBuffer has\n    finish(self):\n        return\n    length Length\n    from_bytes(bytes [in Byte; count]) Buffer: return Buffer(length: size(bytes))\n", test_fixtures::preludes::BYTE_PRELUDE),
    );
    let input = package.write(
        "main.gin",
        r#"use 'arrays' forward, Buffer
#entry
main:
    empty is in 0 to 18446744073709551615: forward('')
    short is in 0 to 18446744073709551615: forward('abc')
    unicode is in 0 to 18446744073709551615: forward('é🦀')
    buffer: Buffer.from_bytes('abcd')
    length is in 0 to 18446744073709551615: buffer.length
    buffer.finish
    return when empty = 0 and short = 3 and unicode = 6 and length = 4 then 0 else 1
"#,
    );
    let executable = package.join("program");
    let result = compile_exe_with_gin_home(input, &executable, package.join("home"));
    require_success(&result, Profile::Debug);
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    let output = Command::new(executable).output().unwrap();
    assert!(output.status.success(), "{output:?}");
}
