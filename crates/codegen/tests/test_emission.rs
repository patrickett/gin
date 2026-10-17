mod support;

#[test]
fn typed_operands_lower_to_constrained_inline_code() {
    let source = test_fixtures::emission::increment_source("x86_64-unknown-linux-gnu");
    let (mlir, flaws) = support::codegen_assembly_to_mlir_text(&source, "emit.gin");
    assert!(flaws.is_empty(), "{flaws:?}\n{mlir}");
    assert!(mlir.contains("llvm.inline_asm"), "{mlir}");
    assert!(mlir.contains("={rax},0,~{cc},~{memory}"), "{mlir}");
    assert!(mlir.contains(".byte 72, 255, 192"), "{mlir}");
}

#[test]
fn aggregate_argument_expression_is_evaluated_once() {
    let source = test_fixtures::emission::increment_source("x86_64-unknown-linux-gnu")
        .replace("    value Word\n    finish(self):", "    value Word\n    extra Word\n    finish(self):")
        .replace("inputs: [value: register]", "inputs: [value: register, extra: [architecture: \"x86_64\", bank: \"general\", index: 1, width: 64]]")
        .replace("Inputs(value: value))", "arguments(value))")
        .replace("#fixed_array", "arguments(value Word) Inputs: return Inputs(value: value, extra: 0)\n\n#fixed_array");
    let (mlir, flaws) = support::codegen_assembly_to_mlir_text(&source, "emit_once.gin");
    assert!(flaws.is_empty(), "{flaws:?}\n{mlir}");
    assert_eq!(mlir.matches("callee = @arguments").count(), 1, "{mlir}");
    assert!(mlir.contains("={rax},0,{rcx},~{cc},~{memory}"), "{mlir}");
}

#[test]
fn multiple_outputs_are_packed_into_the_signature_record() {
    let source = test_fixtures::emission::increment_source("x86_64-unknown-linux-gnu")
        .replace("sig Signature(Inputs, Word):", "Outputs has\n    first Word\n    second Word\n    finish(self):\n        return\nsig Signature(Inputs, Word):")
        .replace("Signature(Inputs, Word)", "Signature(Inputs, Outputs)")
        .replace("outputs: [value: register]", "outputs: [second: [architecture: \"x86_64\", bank: \"general\", index: 1, width: 64], first: register]")
        .replace("increment(value Word) Word:", "increment(value Word) Outputs:");
    let (mlir, flaws) = support::codegen_assembly_to_mlir_text(&source, "emit_outputs.gin");
    assert!(flaws.is_empty(), "{flaws:?}\n{mlir}");
    assert!(mlir.contains("={rax},={rcx},0,~{cc},~{memory}"), "{mlir}");
    let start = mlir.find("sym_name = \"increment\"").unwrap();
    let body = &mlir[start..];
    let end = body.find("\n  \"func.func\"").unwrap_or(body.len());
    let body = &body[..end];
    assert_eq!(body.matches("llvm.insertvalue").count(), 1, "{body}");
    assert_eq!(body.matches("llvm.extractvalue").count(), 1, "{body}");
}

#[test]
fn local_instruction_construction_is_erased_from_runtime_body() {
    let source = test_fixtures::emission::increment_source("x86_64-unknown-linux-gnu");
    let start = source.find("code :=").unwrap();
    let end = source.find("increment(").unwrap();
    let code = source[start..end]
        .lines()
        .map(|line| format!("    {line}\n"))
        .collect::<String>();
    let source = format!(
        "{}increment(value Word) Word:\n{code}    return #emit(code, Inputs(value: value))\n{}",
        &source[..start],
        source
            .split("\n#fixed_array")
            .nth(1)
            .map(|tail| format!("#fixed_array{tail}"))
            .unwrap()
    );
    let (mlir, flaws) = support::codegen_assembly_to_mlir_text(&source, "emit_local.gin");
    assert!(flaws.is_empty(), "{flaws:?}\n{mlir}");
    assert!(!mlir.contains("callee = @instruction"), "{mlir}");
    assert!(mlir.contains(".byte 72, 255, 192"), "{mlir}");
}

#[test]
fn explicit_target_description_is_compile_time_data() {
    let source = test_fixtures::emission::increment_source("x86_64-unknown-linux-gnu")
        .replace("#target.triple", "\"x86_64-unknown-linux-gnu\"");
    let (mlir, flaws) = support::codegen_assembly_to_mlir_text(&source, "emit_target.gin");
    assert!(flaws.is_empty(), "{flaws:?}\n{mlir}");
}
