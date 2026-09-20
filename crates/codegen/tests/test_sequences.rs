mod support;

#[test]
fn sequences_cross_function_boundaries_without_flattening_nested_values() {
    for source in [
        "Byte is in 0 to 255\nget() value Byte: return 42\nmain() Byte: get().value\n",
        "Byte is in 0 to 255\nget() first Byte, second Byte: return 1, 2\nidentity(value [first Byte, second Byte]) [first Byte, second Byte]: value\nmain() Byte: identity(get()).second\n",
        "Byte is in 0 to 255\nget() values [Byte; 2], count Byte: return [1, 2], 2\nmain() Byte: get().values.0\n",
        "Byte is in 0 to 255\nget() outer [first Byte, second Byte], last Byte: return [first: 1, second: 2], 3\nmain() Byte: get().outer.second\n",
        "Byte is in 0 to 255\nget() Byte, Byte: return 1, 2\nmain() Byte: get().1\n",
    ] {
        let (llvm, diagnostics) = support::codegen_to_llvm_ir(source, "sequences.gin");
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}\n{llvm}");
        assert!(llvm.contains("define"), "{llvm}");
    }
}

#[test]
fn stored_sequences_support_mutation_and_dynamic_uniform_indexing() {
    for source in [
        "Byte is in 0 to 255\nmain() Byte:\n    values: x: 1, y: 2\n    values.x:: 3\nreturn values.x as Byte\n",
        "Byte is in 0 to 255\nIndex is in 0 to 1\nmain(value Byte, index Index) Byte:\n    values: x: value, y: value\nreturn values.(index)\n",
    ] {
        let (llvm, diagnostics) = support::codegen_to_llvm_ir(source, "sequence_storage.gin");
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}\n{llvm}");
        assert!(llvm.contains("define"), "{llvm}");
    }
}

#[test]
fn native_sequences_preserve_values_across_calls_and_mutation() {
    use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
    use typecheck::transform::{TransformCtx, transform};

    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    for body in [
        "get() value Word: return 42\nanswer() Word: get().value\n",
        "get() first Word, second Word: return 10, 42\nidentity(value [first Word, second Word]) [first Word, second Word]: value\nanswer() Word: identity(get()).second\n",
        "get() first Byte, second Word: return 10, 42\nanswer() Word: get().second\n",
        "get() values [Word; 2], count Byte: return [10, 42], 2\nanswer() Word: get().values.1\n",
        "get() Word, Word: return 10, 42\nanswer() Word: get().1\n",
        "get() [Word; 2]: return 10, 42\nanswer() Word: get().1\n",
        "answer() Word:\n    values [first Word, second Word]: [10, 42]\nreturn values.second\n",
        "answer() Word:\n    values: first: 10, second: 20\n    values.second:: 42\nreturn values.second as Word\n",
        "Index is in 0 to 1\nselect(index Index) Word:\n    values: first: 10, second: 20\n    values.second:: 42\nreturn values.(index) as Word\nanswer() Word: select(1)\n",
    ] {
        let source = format!(
            "Word is in -9223372036854775808 to 9223372036854775807\nByte is in 0 to 255\n{body}"
        );
        let parsed = parser::parse_from_str(&source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{body}: {:?}",
            parsed.parse_warnings
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let typed = transform(&parsed, typecheck::FileId(0), &scope);
        assert!(
            typed.all_flaws().is_empty(),
            "{body}: {:?}",
            typed.all_flaws()
        );
        let context = NativeCompiler::create_context();
        let (module, flaws) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("native_sequences.gin", &source, &typed.span_table),
            &target,
        );
        assert!(flaws.is_empty(), "{body}: {flaws:?}");
        let engine = NativeEngine::new(module.unwrap(), "answer").unwrap();
        assert_eq!(engine.invoke_i64().unwrap(), 42, "{body}");
    }
}

#[test]
fn invalid_named_assembly_results_report_diagnostics_without_recursing() {
    let source = "Byte is in 0 to 255\nget() value Byte: #asm(42)\n";
    let (_, flaws) = support::codegen_verifies(source, "invalid_sequence_assembly.gin");
    assert!(!flaws.is_empty());
}
