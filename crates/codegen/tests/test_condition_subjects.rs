use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler};
use typecheck::transform::{TransformCtx, transform};

#[test]
fn matched_subject_is_evaluated_once() {
    for body in [
        "    when increment(value) is 42 then\n        return 1\n    return 0\n",
        "    when increment(value) is result then\n        return result\n    return 0\n",
    ] {
        let target_name = test_fixtures::host_target_triple();
        let source = format!(
            "{}\nselect(value Word) Word:\n{body}",
            test_fixtures::emission::increment_source(&target_name).replace(
                "Word has\n",
                "Word has\n    copy(self ref) Self: return deref self\n"
            )
        );
        let parsed = parser::parse_from_str(&source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let target =
            flask::CompileTarget::Concrete(flask::TargetTriple::parse(&target_name).unwrap());
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let typed = transform(&parsed, typecheck::FileId(0), &scope);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("condition.gin", &source, &typed.span_table),
            &target,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let ir = module.unwrap().as_operation().to_string();
        assert_eq!(ir.matches("call @increment(").count(), 1, "{ir}");
    }
}

#[test]
#[allow(unsafe_code)]
fn condition_effects_and_bindings_share_the_evaluated_subject() {
    for (body, expected_result, expected_state) in [
        (
            "    when bump(value) = 42 then\n        return 7\n    return 0\n",
            7,
            42,
        ),
        (
            "    when bump(value) < bump(value) then\n        return 7\n    return 0\n",
            7,
            43,
        ),
        (
            "    when bump(value) in 0 to 42 then\n        return 7\n    return 0\n",
            7,
            42,
        ),
        (
            "    when bump(value) = 0 and bump(value) = 43 then\n        return 7\n    return 0\n",
            0,
            42,
        ),
        (
            "    when bump(value) = 42 or bump(value) = 43 then\n        return 7\n    return 0\n",
            7,
            42,
        ),
        (
            "    when bump(value) = 0 or bump(value) = 43 then\n        return 7\n    return 0\n",
            7,
            43,
        ),
        (
            "    when not (bump(value) = 0) then\n        return 7\n    return 0\n",
            7,
            42,
        ),
        ("    return when bump(value) = 42 then 7 else 0\n", 7, 42),
        (
            "    while bump(value) < 43 do\n        value:: replace(deref value, 43)\n    return deref value\n",
            44,
            44,
        ),
        (
            "    when bump(value) is 42 then\n        return 7\n    return 0\n",
            7,
            42,
        ),
        (
            "    when bump(value) is result then\n        return result\n    return 0\n",
            42,
            42,
        ),
        ("    return when bump(value) is 42 then 7 else 0\n", 7, 42),
        (
            "    when bump(value) is 42 and bump(value) is result then\n        return result\n    return 0\n",
            43,
            43,
        ),
        (
            "    when bump(value) is 0 and bump(value) is result then\n        return result\n    return 0\n",
            0,
            42,
        ),
        (
            "    when bump(value) is 42 or bump(value) is 43 then\n        return 7\n    return 0\n",
            7,
            42,
        ),
        (
            "    when bump(value) is 0 or bump(value) is 43 then\n        return 7\n    return 0\n",
            7,
            43,
        ),
        (
            "    when not (bump(value) is 0) then\n        return 7\n    return 0\n",
            7,
            42,
        ),
        (
            "    while bump(value) is result do\n        when result is 42 then\n            return result\n    return 0\n",
            42,
            42,
        ),
        (
            "    while bump(value) is 42 do\n        value:: replace(deref value, 43)\n    return deref value\n",
            44,
            44,
        ),
        (
            "    when wrap(value) is Some(result) then\n        return result\n    return 0\n",
            42,
            42,
        ),
        (
            "    when wrap(value) is Some(_) then\n        return 7\n    return 0\n",
            7,
            42,
        ),
        (
            "    while wrap(value) is Some(result) do\n        when result is 42 then\n            return result\n    return 0\n",
            42,
            42,
        ),
        (
            "    when pair(value) is [result, ..._] then\n        return result\n    return 0\n",
            42,
            42,
        ),
        ("    adjust(value)\n    return Word.copy(value)\n", 50, 50),
        (
            "    while bump(value) is result and Word.copy(value) is 42 do\n        value:: replace(Word.copy(value), 43)\n    return Word.copy(value)\n",
            44,
            44,
        ),
    ] {
        let target_name = test_fixtures::host_target_triple();
        let source = format!(
            "{}\nreplace(previous Word, next Word) Word: return next\nbump(value mut Word) Word:\n    value:: replace(deref value, increment(Word.copy(value)))\n    return Word.copy(value)\nMaybe(element) is Some(element) or None\nMaybe(element) has\n    finish(self):\n        return\nwrap(value mut Word) Maybe(Word): return Maybe.Some(bump(value))\npair(value mut Word) [Word; 2]: return [bump(value), 99]\nadjust(value mut Word):\n    when bump(value) is 42 then\n        value:: replace(Word.copy(value), 50)\n        return\n    return\nselect(value mut Word) Word:\n{body}",
            test_fixtures::emission::increment_source(&target_name).replace(
                "Word has\n",
                "Word has\n    copy(self ref) Self: return deref self\n"
            )
        );
        let parsed = parser::parse_from_str(&source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{body}: {:?}",
            parsed.parse_warnings
        );
        let target =
            flask::CompileTarget::Concrete(flask::TargetTriple::parse(&target_name).unwrap());
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let typed = transform(&parsed, typecheck::FileId(0), &scope);
        assert!(
            typed.all_flaws().is_empty(),
            "{body}: {:?}",
            typed.all_flaws()
        );
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("condition.gin", &source, &typed.span_table),
            &target,
        );
        assert!(diagnostics.is_empty(), "{body}: {diagnostics:?}");
        let engine = codegen::NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
        let mut state = 41u64;
        let mut state_pointer = &mut state as *mut u64;
        let mut result = 0u64;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut state_pointer as *mut *mut u64).cast(),
                (&mut result as *mut u64).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, expected_result, "{body}");
        assert_eq!(state, expected_state, "{body}");
    }
}
