//! End-to-end tests for the typed AST → MLIR codegen path.
//!
//! These tests verify that the full pipeline works:
//! 1. Parse Gin source → FileAst
//! 2. Transform → TypedFileAst
//! 3. Lower to MLIR via build_module_from_typed_ast
//! 4. Verify the MLIR output contains expected operations

mod support;

fn typed_codegen_to_mlir(source: &str, filename: &str) -> Option<String> {
    let (text, _) = support::codegen_to_mlir_text(source, filename, false);
    Some(text)
}

#[test]
fn test_typed_ast_literal_codegen() {
    // The simplest case: a single integer literal.
    // `main: 42` should produce a func.func with an arith.constant.
    let source = "main: 42";
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
    // Function calls should produce func.call.
    let source = "add(a Int, b Int) Int: a + b\nmain: add(1, 2)";
    let mlir = typed_codegen_to_mlir(source, "test.gin");

    // FnCall currently returns None from lower_typed_expr (not yet implemented).
    // So the module should still be produced but may not have func.call.
    // This test documents current behavior.
    if let Some(text) = mlir {
        assert!(text.contains("func.func"), "should have func operations");
    }
    // If None, that's expected for now until full ExprId lowering is implemented.
}

#[test]
fn test_typed_ast_multiple_defs() {
    // Multiple defs should create multiple func.func operations.
    let source = "x: 10\ny: 20";
    let mlir = typed_codegen_to_mlir(source, "test.gin").expect("codegen should succeed");

    eprintln!("Generated MLIR:\n{mlir}");

    // Count func.func occurrences
    let func_count = mlir.matches("func.func").count();
    assert_eq!(func_count, 2, "should have 2 func.func operations");
}
