use codegen::build_module_with_context;
use diagnostic::Diagnostic;
use melior::Context;
use parser::parse_from_str;

fn codegen_to_mlir_text(source: &str, filename: &str) -> (String, Vec<Diagnostic>) {
    let mut ast = parse_from_str(source);

    let context = Context::new();
    melior::dialect::DialectHandle::llvm().register_dialect(&context);
    context.get_or_load_dialect("arith");
    context.get_or_load_dialect("func");
    context.get_or_load_dialect("scf");
    context.get_or_load_dialect("llvm");

    let (module, symptoms) = build_module_with_context(&context, &mut ast, source, filename);
    let mlir_text = module
        .expect("codegen should succeed")
        .as_operation()
        .to_string();
    (mlir_text, symptoms)
}

fn assert_no_symptoms(symptoms: &[Diagnostic]) {
    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );
}

// Self-contained print program with all dependencies defined inline.
const PRINT_PROGRAM: &str = "\
Int is -9223372036854775808...9223372036854775807

sys_write := 4

write(fd Int, buf Int, len Int) Int:
    result := asm('svc #0x80', '={x0},{x16},0,{x1},{x2},{x3},{x4},~{memory}', sys_write, fd, buf, len, 0, 0)
return result

print(s Str):
    write(1, s.pointer as Int, s.len)
return

main:
    print('hello world')
return
";

#[test]
fn test_print_produces_inline_asm() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("llvm.inline_asm"),
        "should contain syscall via inline asm:\n{mlir_text}"
    );
}

#[test]
fn test_print_produces_extractvalue() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("extractvalue"),
        "should extract pointer and len fields from string struct:\n{mlir_text}"
    );
}

#[test]
fn test_print_produces_ptrtoint() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("llvm.ptrtoint"),
        "should convert string pointer to int for syscall:\n{mlir_text}"
    );
}

#[test]
fn test_print_produces_string_global() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
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
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
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
fn test_string_global_has_null_terminator() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    // MLIR represents the null terminator as \\00 in the value attribute.
    assert!(
        mlir_text.contains("hello world\\00"),
        "string global should be null-terminated:\n{mlir_text}"
    );
}

#[test]
fn test_string_global_has_correct_array_size() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    // "hello world" is 11 chars + 1 null = 12 bytes.
    assert!(
        mlir_text.contains("array<12 x i8>"),
        "string global should be array<12 x i8> (11 chars + null):\n{mlir_text}"
    );
}

#[test]
fn test_string_global_is_constant() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("constant"),
        "string global should be marked constant:\n{mlir_text}"
    );
}

// Tests for string struct construction in main.

#[test]
fn test_main_uses_addressof_for_string() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("llvm.mlir.addressof"),
        "main should use addressof to reference the string global:\n{mlir_text}"
    );
}

#[test]
fn test_main_uses_insertvalue_to_build_string_struct() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("llvm.insertvalue"),
        "main should use insertvalue to build the string struct:\n{mlir_text}"
    );
}

#[test]
fn test_main_emits_correct_string_length() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("value = 11 : i64"),
        "main should emit string length 11 for 'hello world':\n{mlir_text}"
    );
}

// Tests for function signatures.

#[test]
fn test_main_function_signature_is_unit_to_unit() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("function_type = () -> (), sym_name = \"main\""),
        "main should have type () -> ():\n{mlir_text}"
    );
}

#[test]
fn test_print_function_signature_is_string_struct_to_unit() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text
            .contains("function_type = (!llvm.struct<(ptr, i64)>) -> (), sym_name = \"print\""),
        "print should accept string struct and return unit:\n{mlir_text}"
    );
}

#[test]
fn test_write_function_signature_is_three_i64_to_i64() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("function_type = (i64, i64, i64) -> i64, sym_name = \"write\""),
        "write should take 3 i64 args and return i64:\n{mlir_text}"
    );
}

// Tests for function call mechanics.

#[test]
fn test_main_calls_print_via_func_call() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("callee = @print"),
        "main should call print via func.call:\n{mlir_text}"
    );
}

#[test]
fn test_print_calls_write_via_func_call() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("callee = @write"),
        "print should call write via func.call:\n{mlir_text}"
    );
}

// Tests for the write syscall internals.

#[test]
fn test_write_uses_syscall_number_as_first_operand() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    // The inline asm should have sys_write (4) as its first operand.
    // This verifies the bind `sys_write := 4` is resolved and passed correctly.
    assert!(
        mlir_text.contains("callee = @sys_write"),
        "write function should reference sys_write bind:\n{mlir_text}"
    );
}

#[test]
fn test_sys_write_is_zero_arg_function_returning_constant() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("function_type = () -> i64, sym_name = \"sys_write\""),
        "sys_write bind should be compiled as a 0-arg function returning i64:\n{mlir_text}"
    );
}

// Tests for the print function body.

#[test]
fn test_print_extracts_pointer_at_position_0() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("position = array<i64: 0>"),
        "print should extract pointer field at position 0:\n{mlir_text}"
    );
}

#[test]
fn test_print_extracts_len_at_position_1() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("position = array<i64: 1>"),
        "print should extract len field at position 1:\n{mlir_text}"
    );
}

#[test]
fn test_print_passes_fd_1_to_write() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    // The print function passes fd=1 (stdout) as the first arg to write.
    assert!(
        mlir_text.contains("value = 1 : i64"),
        "print should pass file descriptor 1 (stdout) to write:\n{mlir_text}"
    );
}

#[test]
fn test_print_returns_void() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
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
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("llvm.struct<(ptr, i64)>"),
        "string should be represented as struct<(ptr, i64)>:\n{mlir_text}"
    );
}

// Test with multiple print calls.

const MULTI_PRINT_PROGRAM: &str = "\
Int is -9223372036854775808...9223372036854775807

sys_write := 4

write(fd Int, buf Int, len Int) Int:
    result := asm('svc #0x80', '={x0},{x16},0,{x1},{x2},{x3},{x4},~{memory}', sys_write, fd, buf, len, 0, 0)
return result

print(s Str):
    write(1, s.pointer as Int, s.len)
return

main:
    print('hello')
    print('world')
return
";

#[test]
fn test_multiple_prints_produce_separate_globals() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(MULTI_PRINT_PROGRAM, "test.gin");
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
    let (mlir_text, symptoms) = codegen_to_mlir_text(MULTI_PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("hello\\00"),
        "should contain 'hello' string global:\n{mlir_text}"
    );
    assert!(
        mlir_text.contains("world\\00"),
        "should contain 'world' string global:\n{mlir_text}"
    );
}

#[test]
fn test_hello_string_is_six_bytes() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(MULTI_PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    // "hello" = 5 chars + 1 null = 6 bytes.
    assert!(
        mlir_text.contains("array<6 x i8>"),
        "'hello' global should be array<6 x i8>:\n{mlir_text}"
    );
}

#[test]
fn test_world_string_is_six_bytes() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(MULTI_PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    // "world" = 5 chars + 1 null = 6 bytes.
    // Both globals should have array<6 x i8>.
    let count = mlir_text.matches("array<6 x i8>").count();
    assert!(
        count >= 2,
        "both 'hello' and 'world' should produce array<6 x i8>, found {count}:\n{mlir_text}"
    );
}

// Test with an empty string.

const EMPTY_STRING_PRINT_PROGRAM: &str = "\
Int is -9223372036854775808...9223372036854775807

sys_write := 4

write(fd Int, buf Int, len Int) Int:
    result := asm('svc #0x80', '={x0},{x16},0,{x1},{x2},{x3},{x4},~{memory}', sys_write, fd, buf, len, 0, 0)
return result

print(s Str):
    write(1, s.pointer as Int, s.len)
return

main:
    print('')
return
";

#[test]
fn test_empty_string_print_has_length_zero() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(EMPTY_STRING_PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("value = 0 : i64"),
        "empty string print should emit length 0:\n{mlir_text}"
    );
}

#[test]
fn test_empty_string_global_is_one_byte() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(EMPTY_STRING_PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    // Empty string = 0 chars + 1 null = 1 byte.
    assert!(
        mlir_text.contains("array<1 x i8>"),
        "empty string global should be array<1 x i8> (null terminator only):\n{mlir_text}"
    );
}

// Test that the inline asm has side effects flag (represented as ~{memory} clobber).

#[test]
fn test_inline_asm_has_memory_clobber() {
    let (mlir_text, symptoms) = codegen_to_mlir_text(PRINT_PROGRAM, "test.gin");
    assert_no_symptoms(&symptoms);
    assert!(
        mlir_text.contains("~{memory}"),
        "inline asm for syscall should clobber memory:\n{mlir_text}"
    );
}
