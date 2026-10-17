mod support;
use internment::Intern;
use parser::parse_from_str;

const INTEGER_TYPES: &str = "DefaultInteger is in 0 to 255\n";
const STRING_TYPES: &str = r#"Int is in -9223372036854775808 to 9223372036854775807
Byte is in 0 to 255
String has pointer @Byte, len Int
"#;

#[test]
fn test_parse_string_literal() {
    let src = "hello_text: 'hello'\n";
    let ast = parse_from_str(src);
    assert!(ast.defs.contains_key(&Intern::from_ref("hello_text")));
}

#[test]
fn test_compile_number_literal() {
    let source = format!("{INTEGER_TYPES}hello_text: 42\n");
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(&source, "test.gin", false);
    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );
    assert!(!mlir_text.is_empty(), "should produce MLIR output");
}

#[test]
fn test_compile_string_literal() {
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(
        &format!("{STRING_TYPES}hello_text String: String(pointer: (@'hello') as @Byte, len: 5)\n"),
        "test.gin",
        false,
    );
    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );

    assert!(
        mlir_text.contains("llvm.mlir.global"),
        "should contain global: {mlir_text}"
    );
    assert!(
        mlir_text.contains("llvm.mlir.addressof"),
        "should contain addressof: {mlir_text}"
    );
    assert!(
        mlir_text.contains("llvm.insertvalue"),
        "should contain insertvalue: {mlir_text}"
    );
    assert!(
        mlir_text.contains("llvm.struct"),
        "should contain struct type: {mlir_text}"
    );
}

#[test]
fn string_local_passes_to_string_parameter() {
    let source = format!(
        "{STRING_TYPES}\
identity(value String) String:
    return value

entry() String:
    message := String(pointer: (@'hello') as @Byte, len: 5)
    return identity(message)
"
    );
    let (verified, symptoms) = support::codegen_verifies(&source, "string_local.gin");

    assert!(symptoms.is_empty(), "unexpected symptoms: {symptoms:?}");
    assert!(verified, "string local should produce valid MLIR");
}

#[test]
fn test_compile_unterminated_string() {
    // Unterminated strings produce lex errors during parsing.
    // We just verify parse_from_str doesn't panic and produces an AST
    // (possibly default/empty).
    let src = "hello_text: 'hello\n";
    let _ast = parse_from_str(src);
}

#[test]
fn test_compile_lone_quote() {
    let src = "y: '\n";
    let _ast = parse_from_str(src);
}

#[test]
fn test_compile_const_union_string() {
    let src = format!(
        "{INTEGER_TYPES}LogLevel is 'debug' or 'info' or 'warn' or 'error'

main() DefaultInteger:
    level LogLevel: 'debug'
    return 0
"
    );
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(&src, "test_const_union.gin", false);
    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms for ConstUnion: {symptoms:?}"
    );
    assert!(!mlir_text.is_empty(), "should produce MLIR output");
    // The ConstUnion should produce a simple integer constant, not a struct
    assert!(
        mlir_text.contains("arith.constant"),
        "should contain integer constant: {mlir_text}"
    );
}

#[test]
fn test_compile_const_union_function_arg() {
    let src = "LogLevel is 'debug' or 'info' or 'warn' or 'error'

set_log_level(level LogLevel):
    return

main():
    set_log_level('warn' as LogLevel)
    return
";
    let parsed = parse_from_str(src);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let typed = typecheck::transform::transform_file(parsed, typecheck::FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let (mlir_text, symptoms) =
        support::codegen_to_mlir_text(src, "test_const_union_fn.gin", false);
    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms for ConstUnion fn arg: {symptoms:?}"
    );
    assert!(!mlir_text.is_empty(), "should produce MLIR output");
    assert!(mlir_text.contains("func.call"), "should call set_log_level");
    assert!(
        !mlir_text.contains("llvm.insertvalue"),
        "payload-free union should use its integer discriminant representation: {mlir_text}"
    );
}

#[test]
fn test_when_basic_ternary() {
    // Basic single-line ternary with string payloads.
    let src = format!(
        "{INTEGER_TYPES}{STRING_TYPES}\nmain() String: return when 1 < 2 and 2 < 3 then String(pointer: (@'debug') as @Byte, len: 5) else String(pointer: (@'info') as @Byte, len: 4)\n"
    );
    let (mlir_text, symptoms) =
        support::codegen_to_mlir_text(&src, "test_const_union_when.gin", false);
    assert!(symptoms.is_empty(), "expected no symptoms: {symptoms:?}");
    assert!(!mlir_text.is_empty(), "should produce MLIR");
    assert!(
        mlir_text.contains("scf.if"),
        "should have scf.if for ternary: {mlir_text}"
    );
    assert!(
        mlir_text.contains("arith.cmpi"),
        "integer condition should be normalized to i1: {mlir_text}"
    );
    assert!(
        mlir_text.contains("llvm.insertvalue"),
        "string ternary should construct String-compatible results: {mlir_text}"
    );
}

#[test]
fn test_compile_const_union_in_when() {
    // String union results in a `when` expression should route through the shared ABI adapter.
    let src = format!(
        "{STRING_TYPES}LogLevel is 'debug' or 'info' or 'warn' or 'error'

main() String:
    level LogLevel: 'debug'
    result String: when level is
        'debug' then String(pointer: (@'debug') as @Byte, len: 5)
        'info' then String(pointer: (@'info') as @Byte, len: 4)
        else String(pointer: (@'error') as @Byte, len: 5)
    return result
"
    );
    let (mlir_text, symptoms) =
        support::codegen_to_mlir_text(&src, "test_const_union_when.gin", false);
    assert!(
        symptoms.is_empty(),
        "expected no symptoms for const union when: {symptoms:?}"
    );
    assert!(
        mlir_text.contains("llvm.insertvalue"),
        "string union branch results should pass through string ABI compatibility: {mlir_text}"
    );
}
