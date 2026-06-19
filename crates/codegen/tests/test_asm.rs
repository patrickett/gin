use codegen::CodegenContext;
use diagnostic::Diagnostic;
use melior::Context;
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::transform_file;

fn codegen_to_mlir_text(source: &str, filename: &str) -> (String, Vec<Diagnostic>) {
    let ast = TokenCursor::parse_source(source);
    let typed = transform_file(ast, FileId(0));

    let context = Context::new();
    melior::dialect::DialectHandle::llvm().register_dialect(&context);
    context.get_or_load_dialect("arith");
    context.get_or_load_dialect("func");
    context.get_or_load_dialect("scf");
    context.get_or_load_dialect("llvm");

    let (module, symptoms) =
        CodegenContext::build_module_from_typed_ast(&context, &typed, source, filename, None);
    let mlir_text = module
        .expect("codegen should succeed")
        .as_operation()
        .to_string();
    (mlir_text, symptoms)
}

#[test]
fn test_asm_basic() {
    let src = "result := asm('nop', '')\n";
    let (mlir_text, symptoms) = codegen_to_mlir_text(src, "test.gin");

    for symptom in &symptoms {
        eprintln!("codegen symptom: {symptom:?}");
    }

    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );
    assert!(
        mlir_text.contains("llvm.inline_asm"),
        "should contain llvm.inline_asm:\n{mlir_text}"
    );
}

#[test]
fn test_asm_with_operands() {
    let src = "result := asm('mov x0, x1', '={x0},{x1}', 42)\n";
    let (mlir_text, symptoms) = codegen_to_mlir_text(src, "test.gin");

    for symptom in &symptoms {
        eprintln!("codegen symptom: {symptom:?}");
    }

    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );
    assert!(
        mlir_text.contains("llvm.inline_asm"),
        "should contain llvm.inline_asm:\n{mlir_text}"
    );
}

#[test]
fn test_asm_syscall_style() {
    let src = "result := asm('svc #0x80', '={x0},{x16},0,{x1},{x2},{x3},{x4},~{memory}', 4, 1, 0, 5, 0, 0)\n";
    let (mlir_text, symptoms) = codegen_to_mlir_text(src, "test.gin");

    for symptom in &symptoms {
        eprintln!("codegen symptom: {symptom:?}");
    }

    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );
    assert!(
        mlir_text.contains("llvm.inline_asm"),
        "should contain llvm.inline_asm:\n{mlir_text}"
    );
}

#[test]
fn test_asm_in_function() {
    let src = "\
do_thing(x Int) Int:
    result := asm('add x0, x0, x1', '={x0},{x0},{x1}', x, 1)
return result
";
    let (mlir_text, symptoms) = codegen_to_mlir_text(src, "test.gin");

    for symptom in &symptoms {
        eprintln!("codegen symptom: {symptom:?}");
    }

    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );
    assert!(
        mlir_text.contains("llvm.inline_asm"),
        "should contain llvm.inline_asm:\n{mlir_text}"
    );
    assert!(
        mlir_text.contains("do_thing"),
        "should contain the function name:\n{mlir_text}"
    );
}
