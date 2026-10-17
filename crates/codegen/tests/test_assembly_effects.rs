use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, Profile};
use typecheck::assembly::AssemblyState;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TypedExprKind};

#[test]
fn checked_assembly_effects_reach_llvm_inline_assembly() {
    for side_effects in [false, true] {
        let source = test_fixtures::emission::increment_source("x86_64-unknown-linux-gnu");
        let parsed = parser::parse_from_str(&source);
        assert!(parsed.parse_warnings.is_empty());
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap(),
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let mut typed = transform(&parsed, FileId(0), &scope);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let mut count = 0;
        for kind in &mut typed.exprs.kind {
            if let TypedExprKind::Emit {
                state: AssemblyState::Checked(assembly),
                ..
            } = kind
            {
                assert!(assembly.side_effects);
                assembly.side_effects = side_effects;
                count += 1;
            }
        }
        assert_eq!(count, 1);
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("effects.gin", &source, &typed.span_table),
            &target,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let (llvm, diagnostics) =
            NativeCompiler::default().llvm_ir_from_module(&module.unwrap(), Profile::Debug, true);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let llvm = llvm.unwrap();
        assert_eq!(llvm.contains("asm sideeffect"), side_effects, "{llvm}");
        assert_eq!(llvm.contains("~{memory}"), side_effects, "{llvm}");
    }
}
