use codegen::build_module_with_context;
use diagnostic::codegen::CodegenSymptom;
use ast::parse_from_str;
use typeck::TyEnv;
use internment::Intern;
use melior::Context;

/// Helper to generate MLIR text from a source string.
fn codegen_to_mlir_text(source: &str, filename: &str) -> (String, Vec<CodegenSymptom>) {
    let ast = parse_from_str(source);
    let ty_env = TyEnv::from_file_ast(&ast);

    let context = Context::new();
    melior::dialect::DialectHandle::llvm().register_dialect(&context);
    context.get_or_load_dialect("arith");
    context.get_or_load_dialect("func");
    context.get_or_load_dialect("scf");
    context.get_or_load_dialect("llvm");

    let (module, symptoms) = build_module_with_context(&context, &ast, source, filename, &ty_env);
    let mlir_text = module
        .expect("codegen should succeed")
        .as_operation()
        .to_string();
    (mlir_text, symptoms)
}

#[test]
fn test_parse_string_literal() {
    let src = "hello_text: 'hello'\n";
    let ast = parse_from_str(src);
    assert!(
        ast.defs()
            .contains_key(&Intern::new("hello_text".to_string()))
    );
}

#[test]
fn test_compile_number_literal() {
    let (mlir_text, symptoms) = codegen_to_mlir_text("hello_text: 42\n", "test.gin");
    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );
    assert!(!mlir_text.is_empty(), "should produce MLIR output");
}

#[test]
fn test_compile_empty_function() {
    let (mlir_text, symptoms) = codegen_to_mlir_text("hello_text: 42\n", "test.gin");
    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );
    assert!(!mlir_text.is_empty());
}

#[test]
fn test_compile_string_literal() {
    let (mlir_text, symptoms) = codegen_to_mlir_text("hello_text: 'hello'\n", "test.gin");
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
fn test_compile_string_literal_with_print() {
    let (mlir_text, symptoms) = codegen_to_mlir_text("hello_text: 'hello'\n", "test.gin");

    // Print any diagnostics for debugging
    for symptom in &symptoms {
        eprintln!("codegen symptom: {symptom:?}");
    }

    assert!(!mlir_text.is_empty());
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
