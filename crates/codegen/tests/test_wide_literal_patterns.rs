use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use typecheck::transform::{TransformCtx, transform};

#[test]
#[allow(unsafe_code)]
fn unsigned_128_bit_literal_patterns_preserve_all_bits() {
    for expected in [1u128 << 127, u128::MAX] {
        let source = format!(
            "Wide is in 0 to {}\nWide has\n    finish(self):\n        return\n{}select(value Wide) Byte:\n    when value is {expected} then\n        return 1\n    return 0\n",
            expected,
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let parsed = parser::parse_from_str(&source);
        assert!(parsed.parse_warnings.is_empty());
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let typed = transform(&parsed, typecheck::FileId(0), &scope);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("wide-pattern.gin", &source, &typed.span_table),
            &target,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
        for mut input in [
            0,
            (1u128 << 127) - 1,
            1u128 << 127,
            (1u128 << 127) + 1,
            u128::MAX,
        ] {
            let mut result = 255u8;
            unsafe {
                engine.invoke_packed(&mut [
                    (&mut input as *mut u128).cast(),
                    (&mut result as *mut u8).cast(),
                ])
            }
            .unwrap();
            assert_eq!(
                result,
                u8::from(input == expected),
                "input={input}, pattern={expected}"
            );
        }
    }
}
