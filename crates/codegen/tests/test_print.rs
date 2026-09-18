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
return (left + right) as Int
#infix('+')
add_int(lhs Int, rhs Int) Int: add_bits(lhs, rhs)

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

#[test]
fn test_print_produces_extractvalue() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("extractvalue"),
        "should extract pointer and len fields from string struct:\n{mlir_text}"
    );
}

#[test]
fn test_print_produces_ptrtoint() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("llvm.ptrtoint"),
        "should convert string pointer to int for syscall:\n{mlir_text}"
    );
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

// Tests for the string global constant.

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
fn test_string_global_has_correct_array_size() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("array<11 x i8>"),
        "string global should be array<11 x i8> (11 bytes):\n{mlir_text}"
    );
}

#[test]
fn test_string_global_is_constant() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("constant"),
        "string global should be marked constant:\n{mlir_text}"
    );
}

// Tests for string struct construction in main.

#[test]
fn test_main_uses_addressof_for_string() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("llvm.mlir.addressof"),
        "main should use addressof to reference the string global:\n{mlir_text}"
    );
}

#[test]
fn test_main_uses_insertvalue_to_build_string_struct() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("llvm.insertvalue"),
        "main should use insertvalue to build the string struct:\n{mlir_text}"
    );
}

#[test]
fn test_main_emits_correct_string_length() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("value = 11 : i64"),
        "main should emit string length 11 for 'hello world':\n{mlir_text}"
    );
}

// Tests for function signatures.

#[test]
fn test_main_function_returns_process_exit_status() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("function_type = () -> i64, sym_name = \"main\""),
        "main should return an i64 process exit status:\n{mlir_text}"
    );
}

#[test]
fn test_print_function_signature_is_string_struct_to_unit() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text
            .contains("function_type = (!llvm.struct<(ptr, i64)>) -> (), sym_name = \"print\""),
        "print should accept string struct and return unit:\n{mlir_text}"
    );
}

#[test]
fn test_write_function_signature_is_three_i64_to_i64() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("function_type = (i64, i64, i64) -> i64, sym_name = \"write\""),
        "write should take 3 i64 args and return i64:\n{mlir_text}"
    );
}

// Tests for function call mechanics.

#[test]
fn test_main_calls_print_via_func_call() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("callee = @print"),
        "main should call print via func.call:\n{mlir_text}"
    );
}

#[test]
fn test_print_calls_write_via_func_call() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("callee = @write"),
        "print should call write via func.call:\n{mlir_text}"
    );
}

#[test]
fn test_sys_write_is_zero_arg_function_returning_constant() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("function_type = () -> i64, sym_name = \"sys_write\""),
        "sys_write bind should be compiled as a 0-arg function returning i64:\n{mlir_text}"
    );
}

// Tests for the print function body.

#[test]
fn test_print_extracts_pointer_at_position_0() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("position = array<i64: 0>"),
        "print should extract pointer field at position 0:\n{mlir_text}"
    );
}

#[test]
fn test_print_extracts_len_at_position_1() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("position = array<i64: 1>"),
        "print should extract len field at position 1:\n{mlir_text}"
    );
}

#[test]
fn test_print_passes_fd_1_to_write() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    // The print function passes fd=1 (stdout) as the first arg to write.
    assert!(
        mlir_text.contains("value = 1 : i64"),
        "print should pass file descriptor 1 (stdout) to write:\n{mlir_text}"
    );
}

#[test]
fn test_print_returns_void() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    // Look for a void return in the print function body.
    // The print function should end with `func.return` with no operands.
    assert!(
        mlir_text.contains("\"func.return\"()"),
        "print should return void (no return value):\n{mlir_text}"
    );
}

// Tests for the string struct type.

#[test]
fn test_string_struct_type_is_ptr_and_i64() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("llvm.struct<(ptr, i64)>"),
        "string should be represented as struct<(ptr, i64)>:\n{mlir_text}"
    );
}

// Test with multiple print calls.

const MULTI_PRINT_PROGRAM: &str = r#"Int is in -9223372036854775808 to 9223372036854775807
Byte is in 0 to 255
String has pointer @Byte, len Int

add_bits(lhs Int, rhs Int) Int:
    left is in -9223372036854775808 to 9223372036854775807: lhs
    right is in -9223372036854775808 to 9223372036854775807: rhs
return (left + right) as Int
#infix('+')
add_int(lhs Int, rhs Int) Int: add_bits(lhs, rhs)

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
fn test_hello_string_is_five_bytes() {
    let (mlir_text, symptoms) =
        support::codegen_to_mlir_text(MULTI_PRINT_PROGRAM, "test.gin", false);
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("array<5 x i8>"),
        "'hello' global should be array<5 x i8>:\n{mlir_text}"
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
return (left + right) as Int
#infix('+')
add_int(lhs Int, rhs Int) Int: add_bits(lhs, rhs)

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
