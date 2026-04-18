use codegen::build_module_with_context;
use diagnostic::Symptom;
use melior::Context;
use parser::parse_from_str;
use typeck::TyEnv;

fn codegen_to_mlir_text(source: &str, filename: &str) -> (String, Vec<Symptom>) {
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
