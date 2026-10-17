mod support;
use diagnostic::Diagnostic;

fn assert_no_symptoms(symptoms: &[Diagnostic]) {
    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );
}

// Self-contained print program with all dependencies defined inline.
const PRINT_PROGRAM: &str = r#"Int is in -9223372036854775808 to 9223372036854775807
Byte is in 0 to 255
String has pointer @Byte, len Int

add_bits(lhs Int, rhs Int) Int:
    left is in -9223372036854775808 to 9223372036854775807: lhs
    right is in -9223372036854775808 to 9223372036854775807: rhs
    biased: left + right + 9223372036854775808
    bits is in 0 to 18446744073709551615: biased & 18446744073709551615
    return (bits - 9223372036854775808) as Int
#infix('+')
add_int(lhs Int, rhs Int) Int: return add_bits(lhs, rhs)

sys_write Int := 4

write(fd Int, buf Int, len Int) Int:
    result := sys_write + fd + buf + len
    return result

print(s String):
    write(1, s.pointer as Int, s.len)
    return

main:
    print(String(pointer: (@'hello world') as @Byte, len: 11))
    return
"#;

#[rustfmt::skip]
const PRINT_MLIR_CASES: &[(&str, &str, &str)] = &[
    ("test_print_produces_extractvalue", "extractvalue", "should extract pointer and len fields from string struct"),
    ("test_print_produces_ptrtoint", "llvm.ptrtoint", "should convert string pointer to int for syscall"),
    ("test_string_global_has_correct_array_size", "array<11 x i8>", "string global should be array<11 x i8> (11 bytes)"),
    ("test_string_global_is_constant", "constant", "string global should be marked constant"),
    ("test_main_uses_addressof_for_string", "llvm.mlir.addressof", "main should use addressof to reference the string global"),
    ("test_main_uses_insertvalue_to_build_string_struct", "llvm.insertvalue", "main should use insertvalue to build the string struct"),
    ("test_main_emits_correct_string_length", "value = 11 : i64", "main should emit string length 11 for 'hello world'"),
    ("test_print_function_signature_is_string_struct_to_unit", "function_type = (!llvm.struct<(ptr, i64)>) -> (), sym_name = \"print\"", "print should accept string struct and return unit"),
    ("test_write_function_signature_is_three_i64_to_i64", "function_type = (i64, i64, i64) -> i64, sym_name = \"write\"", "write should take 3 i64 args and return i64"),
    ("test_main_calls_print_via_func_call", "callee = @print", "main should call print via func.call"),
    ("test_print_calls_write_via_func_call", "callee = @write", "print should call write via func.call"),
    ("test_sys_write_is_zero_arg_function_returning_constant", "function_type = () -> i64, sym_name = \"sys_write\"", "sys_write bind should be compiled as a 0-arg function returning i64"),
    ("test_print_extracts_pointer_at_position_0", "position = array<i64: 0>", "print should extract pointer field at position 0"),
    ("test_print_extracts_len_at_position_1", "position = array<i64: 1>", "print should extract len field at position 1"),
    ("test_print_passes_fd_1_to_write", "value = 1 : i64", "print should pass file descriptor 1 (stdout) to write"),
    ("test_print_returns_void", "\"func.return\"()", "print should return void (no return value)"),
];

#[test]
fn print_program_emits_expected_mlir() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    for (test_name, needle, explain) in PRINT_MLIR_CASES {
        assert!(
            mlir_text.contains(needle),
            "{test_name}: {explain}:\n{mlir_text}"
        );
    }
}

#[test]
fn test_print_produces_string_global() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("llvm.mlir.global"),
        "should emit string literal as global constant:\n{mlir_text}"
    );
    assert!(
        mlir_text.contains("hello world"),
        "should contain the string literal text:\n{mlir_text}"
    );
}

#[test]
fn test_print_has_all_functions() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("sym_name = \"main\""),
        "should contain main function:\n{mlir_text}"
    );
    assert!(
        mlir_text.contains("sym_name = \"print\""),
        "should contain print function:\n{mlir_text}"
    );
    assert!(
        mlir_text.contains("sym_name = \"write\""),
        "should contain write function:\n{mlir_text}"
    );
}

#[test]
fn test_string_global_has_no_implicit_null_terminator() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        !mlir_text.contains("hello world\\00"),
        "string global should contain only the literal bytes:\n{mlir_text}"
    );
}

#[test]
fn test_entry_function_returns_process_exit_status() {
    let source = PRINT_PROGRAM.replace("main:", "#entry\nmain:");
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(&source, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("function_type = () -> i64, sym_name = \"main\""),
        "main should return an i64 process exit status:\n{mlir_text}"
    );
}

// Test with multiple print calls.

const MULTI_PRINT_PROGRAM: &str = r#"Int is in -9223372036854775808 to 9223372036854775807
Byte is in 0 to 255
String has pointer @Byte, len Int

add_bits(lhs Int, rhs Int) Int:
    left is in -9223372036854775808 to 9223372036854775807: lhs
    right is in -9223372036854775808 to 9223372036854775807: rhs
    biased: left + right + 9223372036854775808
    bits is in 0 to 18446744073709551615: biased & 18446744073709551615
    return (bits - 9223372036854775808) as Int
#infix('+')
add_int(lhs Int, rhs Int) Int: return add_bits(lhs, rhs)

write(fd Int, buf Int, len Int) Int:
    result := fd + buf + len
    return result

print(s String):
    write(1, s.pointer as Int, s.len)
    return

main:
    print(String(pointer: (@'hello') as @Byte, len: 5))
    print(String(pointer: (@'world') as @Byte, len: 5))
    return
"#;

#[test]
fn test_multiple_prints_produce_separate_globals() {
    let (mlir_text, symptoms) =
        support::codegen_to_mlir_text(MULTI_PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    // Each string literal should get its own global.
    let global_count = mlir_text.matches("llvm.mlir.global").count();
    assert!(
        global_count >= 2,
        "should have at least 2 string globals for two print calls, found {global_count}:\n{mlir_text}"
    );
}

#[test]
fn test_multiple_prints_have_distinct_content() {
    let (mlir_text, symptoms) =
        support::codegen_to_mlir_text(MULTI_PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("hello"),
        "should contain 'hello' string global:\n{mlir_text}"
    );
    assert!(
        mlir_text.contains("world"),
        "should contain 'world' string global:\n{mlir_text}"
    );
}

#[test]
fn test_world_string_is_five_bytes() {
    let (mlir_text, symptoms) =
        support::codegen_to_mlir_text(MULTI_PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    let count = mlir_text.matches("array<5 x i8>").count();
    assert!(
        count >= 2,
        "both 'hello' and 'world' should produce array<5 x i8>, found {count}:\n{mlir_text}"
    );
}

// Test with an empty string.

const EMPTY_STRING_PRINT_PROGRAM: &str = r#"Int is in -9223372036854775808 to 9223372036854775807
Byte is in 0 to 255
String has pointer @Byte, len Int

add_bits(lhs Int, rhs Int) Int:
    left is in -9223372036854775808 to 9223372036854775807: lhs
    right is in -9223372036854775808 to 9223372036854775807: rhs
    biased: left + right + 9223372036854775808
    bits is in 0 to 18446744073709551615: biased & 18446744073709551615
    return (bits - 9223372036854775808) as Int
#infix('+')
add_int(lhs Int, rhs Int) Int: return add_bits(lhs, rhs)

sys_write Int := 4

write(fd Int, buf Int, len Int) Int:
    result := sys_write + fd + buf + len
    return result

print(s String):
    write(1, s.pointer as Int, s.len)
    return

main:
    print(String(pointer: (@'') as @Byte, len: 0))
    return
"#;

#[test]
fn test_empty_string_print_has_length_zero() {
    let (mlir_text, symptoms) =
        support::codegen_to_mlir_text(EMPTY_STRING_PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("value = 0 : i64"),
        "empty string print should emit length 0:\n{mlir_text}"
    );
}

#[test]
fn test_empty_string_global_has_zero_bytes() {
    let (mlir_text, symptoms) =
        support::codegen_to_mlir_text(EMPTY_STRING_PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("array<0 x i8>"),
        "empty string global should be array<0 x i8> (no implicit terminator):\n{mlir_text}"
    );
}

#[test]
fn ordinary_main_preserves_its_unit_result() {
    let (mlir, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "main.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir.contains("function_type = () -> (), sym_name = \"main\""),
        "{mlir}"
    );
}
