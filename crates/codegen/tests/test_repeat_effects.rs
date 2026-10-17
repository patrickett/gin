use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use typecheck::transform::{TransformCtx, transform};

#[test]
fn fixed_repeat_counts_reject_runtime_dependencies_and_effects() {
    let fixture = test_fixtures::emission::increment_source(&test_fixtures::host_target_triple())
        .replace(
            "Word has\n",
            "Word has\n    copy(self ref) Self: return deref self\n",
        );
    for body in [
        "make(length Word) [Word; 3]: return [0; length]\n",
        "replace(previous Word, next Word) Word: return next\ncount(state mut Word) Word:\n    state:: replace(deref state, increment(Word.copy(state)))\n    return 3\nmake(state mut Word) [Word; 3]: return [0; count(state)]\n",
    ] {
        let source = format!("{fixture}\n{body}");
        let parsed = parser::parse_from_str(&source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
        );
        let typed = transform(&parsed, typecheck::FileId(0), &scope);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, d)| d.code.slug() == "type-fixed-array-length-mismatch"),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
#[allow(unsafe_code)]
fn repeated_array_seed_runs_once_even_for_an_empty_array() {
    for (length, count, declared) in [0, 1, 3].into_iter().flat_map(|length| {
        [
            (length, length.to_string(), true),
            (length, "extent()".to_owned(), true),
            (length, length.to_string(), false),
        ]
    }) {
        let fixture =
            test_fixtures::emission::increment_source(&test_fixtures::host_target_triple())
                .replace(
                    "Word has\n",
                    "Word has\n    copy(self ref) Self: return deref self\n",
                );
        let result_type = if declared {
            format!(" [Word; {length}]")
        } else {
            String::new()
        };
        let source = format!(
            "{fixture}\nreplace(previous Word, next Word) Word: return next\nbump(value mut Word) Word:\n    value:: replace(deref value, increment(Word.copy(value)))\n    return Word.copy(value)\nextent() Word: return {length}\nmake(state mut Word){result_type}: return [bump(state); {count}]\nconsume(values [Word; {length}]): return\nselect(state mut Word) Word:\n    consume(make(state))\n    return Word.copy(state)\n"
        );
        let parsed = parser::parse_from_str(&source);
        assert!(parsed.parse_warnings.is_empty());
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let typed = transform(&parsed, typecheck::FileId(0), &scope);
        assert!(
            typed.all_flaws().is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("repeat.gin", &source, &typed.span_table),
            &target,
        );
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
        let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
        let mut state = 41u64;
        let mut pointer = &mut state as *mut u64;
        let mut result = 0u64;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut pointer as *mut *mut u64).cast(),
                (&mut result as *mut u64).cast(),
            ])
        }
        .unwrap();
        assert_eq!(
            state, 42,
            "length={length}, count={count}, declared={declared}"
        );
        assert_eq!(
            result, 42,
            "length={length}, count={count}, declared={declared}"
        );
    }
}
