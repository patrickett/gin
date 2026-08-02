mod support;

#[test]
#[ignore = "inline ASM deferred"]
fn test_asm_basic() {
    let src = "result := asm('nop', '')\n";
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(src, "test.gin", false);

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
#[ignore = "inline ASM deferred"]
fn test_asm_with_operands() {
    let src = "result := asm('mov x0, x1', '={x0},{x1}', 42)\n";
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(src, "test.gin", false);

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
#[ignore = "inline ASM deferred"]
fn test_asm_syscall_style() {
    let src = "result := asm('svc #0x80', '={x0},{x16},0,{x1},{x2},{x3},{x4},~{memory}', 4, 1, 0, 5, 0, 0)\n";
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(src, "test.gin", false);

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
#[ignore = "inline ASM deferred"]
fn test_asm_in_function() {
    let src = "\
do_thing(x Int) Int:
    result := asm('add x0, x0, x1', '={x0},{x0},{x1}', x, 1)
return result
";
    let (mlir_text, symptoms) = support::codegen_to_mlir_text(src, "test.gin", false);

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
