use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use typecheck::transform::{TransformCtx, transform};

#[test]
#[allow(unsafe_code)]
fn literal_union_patterns_use_the_selected_discriminant_width() {
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    for count in [2usize, 3, 256, 257] {
        let alternatives = (0..count)
            .map(|index| format!("'v{index}'"))
            .collect::<Vec<_>>()
            .join(" or ");
        let source = format!(
            "Choice is {alternatives}\nChoice has\n    finish(self):\n        return\n{}select(choice Choice) Byte:\n    when choice is 'v{}' then\n        return 1\n    return 0\n",
            test_fixtures::preludes::BYTE_PRELUDE,
            count - 1
        );
        let parsed = parser::parse_from_str(&source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let typed = transform(&parsed, typecheck::FileId(0), &scope);
        assert!(
            typed.all_flaws().is_empty(),
            "count={count}: {:?}",
            typed.all_flaws()
        );
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("discriminant.gin", &source, &typed.span_table),
            &target,
        );
        assert!(diagnostics.is_empty(), "count={count}: {diagnostics:?}");
        let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
        for index in [0, count - 2, count - 1] {
            let mut byte = u8::try_from(index).unwrap_or(0);
            let mut word = u16::try_from(index).unwrap();
            let input = if count <= 256 {
                (&mut byte as *mut u8).cast()
            } else {
                (&mut word as *mut u16).cast()
            };
            let mut result = 0u8;
            unsafe { engine.invoke_packed(&mut [input, (&mut result as *mut u8).cast()]) }.unwrap();
            assert_eq!(
                result,
                u8::from(index == count - 1),
                "count={count}, index={index}"
            );
        }
    }
}
