mod support;

const LOOKUP: &str = include_str!("../../test-fixtures/fixtures/runtime_bounded_lookup.gin");

fn lower(source: &str) -> String {
    support::assert_valid_source(source);
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "dependent_index.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    mlir
}

#[test]
fn a_runtime_length_precondition_removes_the_lookup_bounds_check() {
    let mlir = lower(LOOKUP);
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(!mlir.contains("llvm.intr.trap"), "{mlir}");
}

#[test]
fn an_unproven_runtime_index_keeps_the_bounds_check() {
    let source = LOOKUP.replace("index Count and < view.length", "index Count");
    let mlir = lower(&source);
    assert!(mlir.contains("llvm.intr.trap"), "{mlir}");
}

#[test]
fn an_upper_bound_alone_does_not_prove_a_signed_index_safe() {
    let source = LOOKUP.replace("index Count and", "index is in -128 to 127 and");
    let mlir = lower(&source);
    assert!(mlir.contains("llvm.intr.trap"), "{mlir}");
}

#[test]
fn both_bounds_remove_a_signed_index_check() {
    let source = LOOKUP.replace(
        "index Count and < view.length",
        "index is in -128 to 127 and >= 0 and < view.length",
    );
    let mlir = lower(&source);
    assert!(!mlir.contains("llvm.intr.trap"), "{mlir}");
}

#[test]
fn arithmetic_does_not_inherit_its_operands_bounds_proof() {
    let source = LOOKUP.replace("view.storage.[index]", "view.storage.[index.as_num + 1]");
    let mlir = lower(&source);
    assert!(mlir.contains("llvm.intr.trap"), "{mlir}");
}

#[test]
fn indexing_another_view_does_not_inherit_the_contract() {
    let source = LOOKUP
        .replace(
            "view ref View(Byte), index",
            "view ref View(Byte), other ref View(Byte), index",
        )
        .replace("view.storage.[index]", "other.storage.[index]");
    let mlir = lower(&source);
    assert!(mlir.contains("llvm.intr.trap"), "{mlir}");
}

#[test]
fn taking_an_element_reference_uses_the_same_bounds_proof() {
    let source = LOOKUP.replace(
        "Byte: view.storage.[index]",
        "ref Byte: ref view.storage.[index]",
    );
    let mlir = lower(&source);
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(!mlir.contains("llvm.intr.trap"), "{mlir}");
}

#[test]
fn mutation_before_a_read_keeps_runtime_checking() {
    let source = LOOKUP.replace(
        "Byte: return view.storage.[index]",
        "Byte:\n    position: index\n    position :: Count.replace(position, 255)\n    return view.storage.[position]",
    );
    let mlir = lower(&source);
    assert!(mlir.contains("llvm.intr.trap"), "{mlir}");
}

#[test]
fn debug_llvm_keeps_the_proven_lookup_free_of_bounds_branches() {
    let (llvm, diagnostics) = support::codegen_to_llvm_ir(LOOKUP, "dependent_index.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{llvm}");
    assert!(llvm.contains("getelementptr"), "{llvm}");
    assert!(!llvm.contains("llvm.trap"), "{llvm}");
    assert!(!llvm.contains("icmp"), "{llvm}");
}

#[test]
#[cfg(any(
    all(target_arch = "aarch64", target_os = "macos"),
    all(target_arch = "x86_64", target_os = "linux")
))]
#[allow(unsafe_code)]
fn native_gather_checks_runtime_lengths_before_calling_the_proven_accessor() {
    use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};

    let gather = r#"Score is in 0 to 65535
Score has Numeric
    release(self):
        return

checked_at(view ref View(Byte), index Count) Byte:
    when index.as_num < view.length.as_num then
        return at(view, index)
    return 0

gather(length Count) Score:
    values [Byte; 4]: [10, 20, 30, 40]
    data: ref values.[0; length]
    view: View(storage: data, length: length)
    first is in Byte: checked_at(view, 3)
    second is in Byte: checked_at(view, 0)
    third is in Byte: checked_at(view, 2)
    fourth is in Byte: checked_at(view, 3)
    view.release
    return (first + second + third + fourth) as Score
"#;
    let source = format!("{LOOKUP}\n{gather}");
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    let mut scope = typecheck::transform::TransformCtx::new();
    scope.compile_target = target.clone();
    let parsed = parser::query::SourceParseExt::parse_source_full(source.as_str());
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = typecheck::transform::transform(&parsed.ast, typecheck::FileId(0), &scope);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("dependent_gather.gin", &source, &typed.span_table),
        &target,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let engine = NativeEngine::new_with_export_abi(module.unwrap(), "gather").unwrap();
    for (mut length, expected) in [(0u8, 0u16), (1, 10), (2, 10), (3, 40), (4, 120)] {
        let mut result = 0u16;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut length as *mut u8).cast(),
                (&mut result as *mut u16).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, expected, "length {length}");
    }
}
