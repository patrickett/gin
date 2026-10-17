use ast::{BindValue, DefinitionId, Expr};
use flask::{CompileTarget, TargetTriple};
use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::assembly::AssemblyState;
use typecheck::transform::{
    PackageTransformOptions, TransformCtx, transform, transform_package_with_shared_context,
};
use typecheck::{FileId, TypedExprKind, TypedFileAst};

const SOURCE: &str = r#"
Word is in 0 to 18446744073709551615
Word has
    finish(self):
        return
Inputs has
    value Word
    finish(self):
        return
#phantom(input, output)
Signature(input, output) has
    version Word
    finish(self):
        return
signature Signature(Inputs, Word): Signature(version: 1)
register := when #target.arch is
    'x86_64' then [architecture: 'x86_64', bank: 'general', index: 0, width: 64]
    else [architecture: 'aarch64', bank: 'general', index: 0, width: 64]
code := when #target.arch is
    'x86_64' then [
        signature: signature, target: #target.triple, bytes: [72, 255, 192]
        inputs: [value: register], outputs: [value: register]
        clobbers: [], reads: [], writes: []
    ]
    else [
        signature: signature, target: #target.triple, bytes: [0, 4, 0, 145]
        inputs: [value: register], outputs: [value: register]
        clobbers: [], reads: [], writes: []
    ]
increment(value Word) Word: return #emit(code, Inputs(value: value))
"#;

fn prepare(source: &str, target: &CompileTarget) -> ast::FileAst {
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut files = [parsed.ast];
    let diagnostics = typecheck::prepare_package_asts(&mut files, target);
    assert!(diagnostics.iter().all(Vec::is_empty), "{diagnostics:?}");
    files.into_iter().next().unwrap()
}

fn check(source: &str, target: &str) -> TypedFileAst {
    let mut context = TransformCtx::new();
    context.compile_target = CompileTarget::Concrete(TargetTriple::parse(target).unwrap());
    let parsed = prepare(source, &context.compile_target);
    transform(&parsed, FileId(0), &context)
}

#[test]
fn concrete_target_selects_descriptor_shape_before_emission_validation() {
    for (target, template, result) in [
        ("x86_64-unknown-linux-gnu", ".byte 72, 255, 192", "rax"),
        ("aarch64-apple-darwin", ".byte 0, 4, 0, 145", "x0"),
    ] {
        let typed = check(SOURCE, target);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let assembly = typed
            .exprs
            .kind
            .iter()
            .find_map(|kind| match kind {
                TypedExprKind::Emit {
                    state: AssemblyState::Checked(assembly),
                    ..
                } => Some(assembly),
                _ => None,
            })
            .expect("validated target-selected emission");
        assert_eq!(assembly.template, template);
        assert_eq!(assembly.result.as_deref(), Some(result));
        assert!(assembly.inputs[0].tie);
    }
}

#[test]
fn per_file_preparation_selects_descriptors_with_cross_file_signatures() {
    let (declarations, descriptor) = SOURCE.split_once("register :=").unwrap();
    let target = CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let sources = [declarations.to_owned(), format!("register :={descriptor}")];
    for reverse in [false, true] {
        let mut files = sources
            .iter()
            .map(|source| {
                let parsed = source.parse_source_full();
                assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
                let mut ast = parsed.ast;
                assert!(typecheck::prepare_parse_ast(&mut ast, &target).is_empty());
                ast
            })
            .collect::<Vec<_>>();
        if reverse {
            files.reverse();
        }
        let artifacts = transform_package_with_shared_context(
            files,
            PackageTransformOptions::FULL.with_compile_target(target.clone()),
        );
        let flaws = artifacts
            .typed_asts
            .iter()
            .flat_map(TypedFileAst::all_flaws)
            .collect::<Vec<_>>();
        assert!(flaws.is_empty(), "{flaws:?}");
        assert!(artifacts.typed_asts.iter().any(|typed| {
            typed.exprs.kind.iter().any(|kind| {
                matches!(
                    kind,
                    TypedExprKind::Emit { state: AssemblyState::Checked(assembly), .. }
                        if assembly.template == ".byte 72, 255, 192"
                )
            })
        }));
    }
}

#[test]
fn selected_descriptors_still_require_valid_signatures_bytes_and_registers() {
    for (before, after, message) in [
        ("signature: signature", "signature: 1", "signature"),
        ("bytes: [72, 255, 192]", "bytes: [256]", "bytes"),
        (
            "architecture: 'x86_64'",
            "architecture: 'aarch64'",
            "architecture",
        ),
    ] {
        let typed = check(&SOURCE.replace(before, after), "x86_64-unknown-linux-gnu");
        assert!(
            typed.all_flaws().iter().any(|(_, diagnostic)| {
                diagnostic.code.slug() == "type-invalid-assembly"
                    && diagnostic.message.contains(message)
            }),
            "{after}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn library_preparation_preserves_all_target_alternatives() {
    let parsed = prepare(SOURCE, &CompileTarget::Library);
    for name in ["register", "code"] {
        let declaration = &parsed.defs[&DefinitionId::named(Intern::from_ref(name))];
        let BindValue::Expr(expression) = &declaration.value else {
            panic!("target-selected expression");
        };
        let Expr::When(when) = &expression.value else {
            panic!("library must defer target selection");
        };
        assert!(when.subject.as_ref().unwrap().const_value.is_none());
        assert_eq!(when.arms.len(), 2);
    }
}

#[test]
fn library_validates_the_common_signature_and_defers_target_encoding() {
    let parsed = prepare(SOURCE, &CompileTarget::Library);
    let typed = transform(&parsed, FileId(0), &TransformCtx::new());
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let emissions = typed
        .exprs
        .kind
        .iter()
        .filter_map(|kind| match kind {
            TypedExprKind::Emit { state, .. } => Some(state),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(emissions.len(), 1);
    assert_eq!(emissions[0], &AssemblyState::Pending);
}

#[test]
fn library_rejects_missing_or_inconsistent_signatures_in_target_alternatives() {
    for (replacement, declarations) in [
        ("invalid_signature: signature", ""),
        ("signature: 1", ""),
        (
            "signature: different",
            "OtherInputs has value Word\ndifferent Signature(OtherInputs, Word): \
             Signature(version: 1)\n",
        ),
        (
            "signature: different",
            "OtherWord is in 0 to 255\ndifferent Signature(Inputs, OtherWord): \
             Signature(version: 1)\n",
        ),
    ] {
        let source = SOURCE.replacen("signature: signature", replacement, 1) + declarations;
        let parsed = prepare(&source, &CompileTarget::Library);
        let typed = transform(&parsed, FileId(0), &TransformCtx::new());
        assert!(
            typed.all_flaws().iter().any(|(_, diagnostic)| {
                diagnostic.code.slug() == "type-invalid-assembly"
                    && diagnostic.message.contains("signature")
            }),
            "{replacement} {declarations}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn known_target_type_selections_remain_available_for_exhaustiveness_validation() {
    let source = concat!(
        "Byte is in 0 to 255\n",
        "Selected is when #target.arch is\n",
        "    'x86_64' then Byte\n",
    );
    let target = CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut file = parsed.ast;
    assert!(typecheck::prepare_parse_ast(&mut file, &target).is_empty());
    assert!(matches!(
        file.tags[&Intern::from_ref("Selected")].value,
        ast::declare::DeclareValue::When(_)
    ));
    let diagnostics = typecheck::prepare_package_asts(&mut [file], &target);
    assert!(
        diagnostics[0]
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "type-missing-else-arm")
    );
}

#[test]
fn target_preparation_does_not_evaluate_runtime_subjects_using_global_names() {
    let source = concat!(
        "architecture: #target.arch\n",
        "select(architecture): return when architecture is\n",
        "    'x86_64' then [value: 1]\n",
        "    else [value: 2, extra: 3]\n",
    );
    let target = CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let parsed = prepare(source, &target);
    let declaration = &parsed.defs[&DefinitionId::named(Intern::from_ref("select"))];
    let BindValue::Body { ret, .. } = &declaration.value else {
        panic!("function body");
    };
    let Expr::When(when) = &ret.values[0].value else {
        panic!("runtime selection must remain");
    };
    assert!(when.subject.as_ref().unwrap().const_value.is_none());
    assert_eq!(when.arms.len(), 2);
}
