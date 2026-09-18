mod support;

#[test]
fn checked_assembly_lowers_to_inline_asm_with_target_and_constraints() {
    let source = r#"Byte is in 0 to 255
protocol := 'gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ntemplate \"nop\"'
write(): #asm(protocol)
"#;
    let (mlir, diagnostics) = support::codegen_assembly_to_mlir_text(source, "assembly.gin");
    assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    assert!(mlir.contains("llvm.inline_asm"), "MLIR: {mlir}");
    assert!(mlir.contains("llvm.target_triple"), "MLIR: {mlir}");
    assert!(mlir.contains("~{memory}"), "MLIR: {mlir}");
    assert!(mlir.contains("nop"), "MLIR: {mlir}");
}

#[test]
fn checked_assembly_lowers_a_scalar_result_register() {
    let source = r#"Byte is in 0 to 255
write() Byte: #asm('gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\nresult fixed \"rax\"\ntemplate \"mov rax, 1\"')
"#;
    let (mlir, diagnostics) = support::codegen_assembly_to_mlir_text(source, "assembly.gin");
    assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    assert!(mlir.contains("llvm.inline_asm"), "MLIR: {mlir}");
    assert!(mlir.contains("={rax}"), "MLIR: {mlir}");
    assert!(mlir.contains("func.return"), "MLIR: {mlir}");
}

#[test]
fn checked_assembly_preserves_named_outputs_as_ssa_results() {
    let source = r#"Word is in 0 to 18446744073709551615
write(value Word) sum Word, overflow Word: #asm('gin-asm 1\ntarget "x86_64-unknown-linux-gnu"\ndialect intel\ninput "value" fixed "rdi"\noutput "sum" fixed "rax"\noutput "overflow" fixed "rdx"\ntemplate "add"', value: value)
"#;
    let (mlir, diagnostics) =
        support::codegen_assembly_to_mlir_text(source, "assembly-results.gin");
    assert!(
        diagnostics.is_empty(),
        "diagnostics: {diagnostics:?}\n{mlir}"
    );
    assert!(mlir.contains("llvm.inline_asm"), "{mlir}");
    assert!(mlir.contains("(i64, i64)"), "{mlir}");
    assert!(!mlir.contains("llvm.insertvalue"), "{mlir}");
}

#[test]
fn checked_assembly_lowers_for_arm64_target() {
    let source = r#"write(): #asm('gin-asm 1\ntarget \"aarch64-unknown-linux-gnu\"\ndialect att\ntemplate \"nop\"')
"#;
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse("aarch64-unknown-linux-gnu").expect("target"),
    );
    let (mlir, diagnostics) =
        support::codegen_assembly_to_mlir_text_for_target(source, "assembly-arm64.gin", target);
    assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    assert!(mlir.contains("llvm.inline_asm"), "MLIR: {mlir}");
    assert!(mlir.contains("aarch64-unknown-linux-gnu"), "MLIR: {mlir}");
    assert!(mlir.contains("~{memory}"), "MLIR: {mlir}");
}

#[test]
fn checked_assembly_lowers_for_arm64_darwin_target() {
    let source = r#"write(): #asm('gin-asm 1\ntarget \"aarch64-apple-darwin\"\ndialect att\nclobber \"x16\"\nclobber \"cc\"\noption side_effects\ntemplate \"mov x16, 4\\nsvc #0x80\"')
"#;
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse("aarch64-apple-darwin").expect("target"),
    );
    let (mlir, diagnostics) =
        support::codegen_assembly_to_mlir_text_for_target(source, "assembly-darwin.gin", target);
    assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    assert!(mlir.contains("llvm.inline_asm"), "MLIR: {mlir}");
    assert!(mlir.contains("aarch64-apple-darwin"), "MLIR: {mlir}");
    assert!(
        mlir.contains("x16")
            && mlir.contains("~{cc}")
            && mlir.contains("~{memory}")
            && mlir.contains("svc #0x80"),
        "MLIR: {mlir}"
    );
}

#[test]
fn checked_assembly_binds_named_runtime_inputs() {
    let source = r#"Word is in 0 to 18446744073709551615
write(value Word): #asm('gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"value\" fixed \"rdi\"\ntemplate \"nop\"', value: value)
"#;
    let (mlir, diagnostics) = support::codegen_assembly_to_mlir_text(source, "assembly-input.gin");
    assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    assert!(mlir.contains("llvm.inline_asm"), "MLIR: {mlir}");
    assert!(mlir.contains("{rdi}"), "MLIR: {mlir}");
    assert!(mlir.contains("%arg"), "MLIR: {mlir}");
}

#[test]
fn checked_assembly_preserves_multiline_template_text() {
    let source = r#"write(): #asm('gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ntemplate \"mov rax, 1\\nsyscall\"')
"#;
    let (mlir, diagnostics) =
        support::codegen_assembly_to_mlir_text(source, "assembly-template.gin");
    assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    assert!(
        mlir.contains("mov rax, 1") && mlir.contains("syscall"),
        "MLIR: {mlir}"
    );
}

#[test]
fn checked_assembly_emits_a_native_object() {
    let source = r#"write(): #asm('gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ntemplate \"nop\"')
"#;
    let object = std::env::temp_dir().join(format!("gin-assembly-{}.o", std::process::id()));
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse("x86_64-unknown-linux-gnu").expect("target"),
    );
    let (ok, diagnostics) =
        support::codegen_assembly_to_object(source, "assembly-object.gin", target, &object);
    assert!(ok, "diagnostics: {diagnostics:?}");
    assert!(object.is_file(), "object was not emitted: {object:?}");
    let _ = std::fs::remove_file(object);
}

#[test]
fn invalid_checked_assembly_does_not_fall_back_to_string_returning_code() {
    let source = "write(): #asm('not a gin assembly contract')\n";
    let (mlir, _) = support::codegen_assembly_to_mlir_text(source, "invalid-assembly.gin");
    assert!(
        !mlir.contains("llvm.inline_asm"),
        "invalid assembly was lowered: {mlir}"
    );
}
