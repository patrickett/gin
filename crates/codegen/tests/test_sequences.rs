mod support;

#[test]
fn sequences_cross_function_boundaries_without_flattening_nested_values() {
    for source in [
        format!(
            "{}get() value Byte: return 42\nmain() Byte: return get().value\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}get() first Byte, second Byte: return 1, 2\nidentity(value [first Byte, second Byte]) [first Byte, second Byte]: return value\nmain() Byte: return identity(get()).second\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}get() values [Byte; 2], count Byte: return [1, 2], 2\nmain() Byte: return get().values.0\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}get() outer [first Byte, second Byte], last Byte: return [first: 1, second: 2], 3\nmain() Byte: return get().outer.second\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}get() Byte, Byte: return 1, 2\nmain() Byte: return get().1\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
    ] {
        let (llvm, diagnostics) = support::codegen_to_llvm_ir(&source, "sequences.gin");
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}\n{llvm}");
        assert!(llvm.contains("define"), "{llvm}");
    }
}

#[test]
fn stored_sequences_support_mutation_and_dynamic_uniform_indexing() {
    for source in [
        format!(
            "{}main() Byte:\n    values: x: 1, y: 2\n    values.x:: 3\n    return values.x as Byte\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}Index is in 0 to 1\nIndex has\n    finish(self):\n        return\nmain(value Byte, index Index) Byte:\n    values: x: value, y: value\n    return values.[index]\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
    ] {
        let (llvm, diagnostics) = support::codegen_to_llvm_ir(&source, "sequence_storage.gin");
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}\n{llvm}");
        assert!(llvm.contains("define"), "{llvm}");
    }
}

#[test]
#[allow(unsafe_code)]
fn native_sequences_preserve_values_across_calls_and_mutation() {
    use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
    use typecheck::transform::{TransformCtx, transform};

    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    for body in [
        "get() value Word: return 42\nanswer() Word: return get().value\n",
        "get() first Word, second Word: return 10, 42\nidentity(value [first Word, second Word]) [first Word, second Word]: return value\nanswer() Word: return identity(get()).second\n",
        "get() first Byte, second Word: return 10, 42\nanswer() Word: return get().second\n",
        "get() values [Word; 2], count Byte: return [10, 42], 2\nanswer() Word: return get().values.1\n",
        "get() Word, Word: return 10, 42\nanswer() Word: return get().1\n",
        "get() [Word; 2]: return 10, 42\nanswer() Word: return get().1\n",
        "get(flag Byte) [Word; 2]:\n    when flag is 0 then\n        return 10, 42\n    return 11, 41\nanswer() Word: return get(0).1\n",
        "get(flag Byte) [Word; 2]:\n    when flag is 0 then\n        return 10, 41\n    return 11, 42\nanswer() Word: return get(1).1\n",
        "answer() Word:\n    values [first Word, second Word]: [10, 42]\n    return values.second\n",
        "answer() Word:\n    values: first: 10, second: 20\n    values.second:: 42\n    return values.second as Word\n",
        "Index is in 0 to 1\nIndex has\n    finish(self):\n        return\nselect(index Index) Word:\n    values: first: 10, second: 20\n    values.second:: 42\n    return values.[index] as Word\nanswer() Word: return select(1)\n",
    ] {
        let source = format!(
            "Word is in -9223372036854775808 to 9223372036854775807\nWord has\n    finish(self):\n        return\n{}{body}",
            test_fixtures::preludes::BYTE_PRELUDE
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
        assert_eq!(unsafe { engine.invoke_i64() }.unwrap(), 42, "{body}");
    }
}

#[test]
fn invalid_named_assembly_results_report_diagnostics_without_recursing() {
    let source = format!(
        "{}get() value Byte: return #emit(42, [])\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let (_, flaws) = support::codegen_verifies(&source, "invalid_sequence_assembly.gin");
    assert!(!flaws.is_empty());
}
