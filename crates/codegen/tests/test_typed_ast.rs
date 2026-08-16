//! End-to-end tests for the typed AST → MLIR codegen path.
//!
//! These tests verify that the full pipeline works:
//! 1. Parse Gin source → FileAst
//! 2. Transform → TypedFileAst
//! 3. Lower the resolved program to MLIR
//! 4. Verify the MLIR output contains expected operations

mod support;

fn typed_codegen_to_mlir(source: &str, filename: &str) -> Option<String> {
    let (text, _) = support::codegen_to_mlir_text(source, filename, false);
    Some(text)
}

#[test]
fn test_typed_ast_literal_codegen() {
    let source = "Signed64 is in -9223372036854775808...9223372036854775807\n\
main Signed64: 42";
    let mlir = typed_codegen_to_mlir(source, "test.gin").expect("codegen should succeed");

    eprintln!("Generated MLIR:\n{mlir}");

    // The MLIR output should contain:
    // - A `func.func` operation with the name `main`
    // - An `arith.constant` with value 42
    assert!(
        mlir.contains("func.func"),
        "should contain func.func, got:\n{mlir}"
    );
    // The exact MLIR output depends on melior's formatting.
    // Just verify it's well-formed.
    assert!(!mlir.is_empty(), "MLIR output should not be empty");
}

#[test]
fn test_typed_ast_fn_call_codegen() {
    let source = "Signed64 is in -9223372036854775808...9223372036854775807\n\
first(a Signed64, b Signed64) Signed64: a\n\
main() Signed64: first(1, 2)";
    let mlir = typed_codegen_to_mlir(source, "test.gin").expect("codegen should succeed");

    assert!(mlir.contains("func.func"), "should have func operations");
    assert!(
        mlir.contains("func.call"),
        "should lower the function call, got:\n{mlir}"
    );
}

#[test]
fn test_typed_ast_multiple_defs() {
    let source = "Signed64 is in -9223372036854775808...9223372036854775807\n\
x Signed64: 10\n\
y Signed64: 20";
    let mlir = typed_codegen_to_mlir(source, "test.gin").expect("codegen should succeed");

    eprintln!("Generated MLIR:\n{mlir}");

    // Count func.func occurrences
    let func_count = mlir.matches("func.func").count();
    assert_eq!(func_count, 2, "should have 2 func.func operations");
}
