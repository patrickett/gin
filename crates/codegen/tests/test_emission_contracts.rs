use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler};
use parser::parse_from_str;
use typecheck::assembly::AssemblyState;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{ExprId, FileId, TypedExprKind};

#[derive(Clone, Copy, Debug)]
enum Corruption {
    MissingOperand,
    MissingTie,
    InvalidTie,
    WrongTiedRegister,
    InvalidProjectionIndex,
    WrongParameter,
    WrongProjectionBase,
    InvalidProjection,
    MissingOutput,
    WrongOutputName,
    ReorderedOutputNames,
}

#[test]
fn inconsistent_checked_assembly_is_rejected_at_the_emission_boundary() {
    for corruption in [
        Corruption::MissingOperand,
        Corruption::MissingTie,
        Corruption::InvalidTie,
        Corruption::WrongTiedRegister,
        Corruption::InvalidProjectionIndex,
        Corruption::WrongParameter,
        Corruption::WrongProjectionBase,
        Corruption::InvalidProjection,
        Corruption::MissingOutput,
        Corruption::WrongOutputName,
        Corruption::ReorderedOutputNames,
    ] {
        let mut source = test_fixtures::emission::increment_source("x86_64-unknown-linux-gnu");
        if matches!(
            corruption,
            Corruption::WrongOutputName | Corruption::ReorderedOutputNames
        ) {
            source = source
                .replace("sig Signature(Inputs, Word):", "Outputs has\n    first Word\n    second Word\n    finish(self):\n        return\nsig Signature(Inputs, Word):")
                .replace("Signature(Inputs, Word)", "Signature(Inputs, Outputs)")
                .replace("outputs: [value: register]", "outputs: [second: [architecture: \"x86_64\", bank: \"general\", index: 1, width: 64], first: register]")
                .replace("increment(value Word) Word:", "increment(value Word) Outputs:");
        }
        let parsed = parse_from_str(&source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap(),
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let mut typed = transform(&parsed, FileId(0), &scope);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let index = typed
            .exprs
            .kind
            .iter()
            .position(|kind| {
                matches!(
                    kind,
                    TypedExprKind::Emit {
                        state: AssemblyState::Checked(_),
                        ..
                    }
                )
            })
            .expect("checked emission");
        let span = typed.span_table.get(typed.exprs.span[index]);
        let TypedExprKind::Emit {
            state: AssemblyState::Checked(assembly),
            ..
        } = &mut typed.exprs.kind[index]
        else {
            unreachable!();
        };
        match corruption {
            Corruption::WrongOutputName => {
                assembly.outputs[0].name = "missing".into();
            }
            Corruption::ReorderedOutputNames => {
                assembly.outputs[0].name = "second".into();
                assembly.outputs[1].name = "first".into();
            }
            Corruption::MissingOperand => {
                assembly.operands.clear();
            }
            Corruption::MissingTie => {
                assembly.tied_output_indices.clear();
            }
            Corruption::InvalidTie => {
                assembly.tied_output_indices[0] = Some(1);
            }
            Corruption::WrongTiedRegister => {
                assembly.inputs[0].register = "rcx".into();
            }
            Corruption::InvalidProjectionIndex => {
                let operand = assembly.operands[0].1;
                let TypedExprKind::AggregateGet { index, .. } =
                    &mut typed.exprs.kind[operand.as_usize()]
                else {
                    panic!("projection");
                };
                *index = usize::MAX;
            }
            Corruption::WrongParameter => {
                assembly.inputs[0].parameter = "other".into();
            }
            Corruption::MissingOutput => {
                assembly.result = None;
            }
            Corruption::WrongProjectionBase | Corruption::InvalidProjection => {
                let operand = assembly.operands[0].1;
                let kind = &mut typed.exprs.kind[operand.as_usize()];
                if matches!(corruption, Corruption::InvalidProjection) {
                    *kind = TypedExprKind::Invalid;
                } else {
                    let TypedExprKind::AggregateGet { base, .. } = kind else {
                        panic!("projection");
                    };
                    *base = ExprId(u32::MAX);
                }
            }
        }
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("emission_contract.gin", &source, &typed.span_table),
            &target,
        );
        assert!(module.is_none(), "{corruption:?}");
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.code.slug()
                == "codegen-assembly-invalid"
                && diagnostic.span == span),
            "{corruption:?}: {diagnostics:?}"
        );
    }
}
