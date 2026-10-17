use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use typecheck::transform::{TransformCtx, transform};

#[test]
fn nonnumeric_negation_is_rejected_before_emission() {
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    for source in [
        "select: -'abc'\n",
        "Flag is Left or Right\nFlag has\n    finish(self):\n        return\nselect(value Flag) Flag: return -value\n",
    ] {
        let parsed = parser::parse_from_str(source);
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
            "{source}: {:?}",
            typed.all_flaws()
        );
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("negation.gin", source, &typed.span_table),
            &target,
        );
        assert!(module.is_none());
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-negation-type-mismatch"),
            "{diagnostics:?}"
        );
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-invalid-module")
        );
    }
}

#[test]
#[allow(unsafe_code)]
fn float_negation_preserves_sign_and_magnitude() {
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    for (source, expected) in [
        ("select: -1.5\n", -1.5f64),
        ("select: -0.0\n", -0.0),
        ("seed: 1.5\nselect: -seed\n", -1.5),
    ] {
        let parsed = parser::parse_from_str(source);
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
            "{source}: {:?}",
            typed.all_flaws()
        );
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("negation.gin", source, &typed.span_table),
            &target,
        );
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
        let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
        let mut result = 0f64;
        unsafe { engine.invoke_packed(&mut [(&mut result as *mut f64).cast()]) }.unwrap();
        assert_eq!(result.to_bits(), expected.to_bits(), "{source}");
    }
}

#[test]
#[allow(unsafe_code)]
fn integer_negation_preserves_selected_storage() {
    let source = "Signed is in -128 to 127\nSigned has\n    finish(self):\n        return\nselect(value Signed) Signed: return -value\n";
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    let parsed = parser::parse_from_str(source);
    assert!(parsed.parse_warnings.is_empty());
    let mut scope = TransformCtx::new();
    scope.compile_target = target.clone();
    let typed = transform(&parsed, typecheck::FileId(0), &scope);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("integer_negation.gin", source, &typed.span_table),
        &target,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
    for mut input in [-127i8, -1, 0, 1, 127] {
        let mut result = 0i8;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut input as *mut i8).cast(),
                (&mut result as *mut i8).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, -input);
    }
}
