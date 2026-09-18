use std::io::Write;
use std::process::{Command, Stdio};

use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn address_store_then_load_preserves_scalar_and_does_not_touch_other_slot() {
    let package = TempPackage::new("typed_memory_runtime");
    let input = package.write(
        "main.gin",
        &test_fixtures::with_runtime(r#"Word is in 0 to 18446744073709551615
PointerToWord is @Word
read_addr(address PointerToWord) Word: address.(0)
copy_pointer(value ref PointerToWord) PointerToWord: deref value
replace(previous Word, next Word) Word: next
write_addr(address PointerToWord, value Word):
    pointer PointerToWord: address
    pointer.(0):: replace(pointer.(0), value)
return
runtime_read_int() Word: runtime_word_bits(runtime_number) as Word
runtime_print_int(value Word) Word: runtime_word_bits(runtime_print(runtime_signed_bits(value as RuntimeWord))) as Word
roundtrip(first Word, second Word, replacement Word) Word:
    first_address PointerToWord: @first
    second_address PointerToWord: @second
    write_addr(copy_pointer(ref first_address), replacement)
    runtime_print_int(read_addr(first_address))
    runtime_print_int(read_addr(second_address))
return 0

main:
    roundtrip(runtime_read_int, runtime_read_int, runtime_read_int)
return 0
"#),
    );
    let output = package.join("typed_memory_runtime");
    let mut args = Args {
        input: input.parent().unwrap().to_path_buf(),
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
        "typed memory compilation failed: {result:?}"
    );
    assert!(
        !result.has_typecheck_diagnostics(),
        "typed memory compilation emitted diagnostics: {result:?}"
    );
    let mut process = Command::new(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run typed memory example");
    process
        .stdin
        .take()
        .unwrap()
        .write_all(b"1\n2\n9\n")
        .unwrap();
    let output = process
        .wait_with_output()
        .expect("wait for typed memory example");
    assert!(output.status.success(), "typed memory example failed");
    assert_eq!(output.stdout, b"9\n2\n");
}
