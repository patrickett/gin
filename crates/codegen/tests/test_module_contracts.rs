use ast::ty::{SequenceTypeElement, Ty};
use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, ResolvedFileInput};
use internment::Intern;
use typecheck::{DefId, FileId, TypedFileAst};

use test_fixtures::host_compile_target as target;

fn typed(source: &str, file: u32) -> TypedFileAst {
    let parsed = parser::parse_from_str(source);
    assert!(parsed.parse_warnings.is_empty());
    let typed = typecheck::transform::transform_file(parsed, FileId(file));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    typed
}

fn add_unrepresentable_parameter(typed: &mut TypedFileAst, name: &str, ty: Ty) {
    let bind = typed
        .defs
        .get_mut(&DefId::named(Intern::from_ref(name)))
        .unwrap();
    let mut params = bind.params.to_vec();
    params.push((Intern::from_ref("broken"), ty.clone()));
    bind.params = params.into();
    bind.param_kinds
        .push(ast::ty::ParamKind::Value(Box::new(ty)));
    bind.param_conventions.push(ast::ParamConvention::Terminal);
}

fn unavailable_type() -> Ty {
    let name = Intern::from_ref("Missing");
    Ty::Named {
        instance: ast::ty::NamedTypeInstance::new(ast::ty::TypeId { file: 0, name }),
        name,
    }
}

#[test]
fn representation_flaws_cannot_return_a_successful_module() {
    for (ty, code) in [
        (
            Ty::Literal(ast::ConstValue::Int(1.into())),
            "literal-representation-unavailable",
        ),
        (
            Ty::Literal(ast::ConstValue::Float(ast::HashFloat(1.5))),
            "literal-representation-unavailable",
        ),
        (
            Ty::Literal(ast::ConstValue::Record { fields: [].into() }),
            "literal-representation-unavailable",
        ),
        (
            Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer),
            "literal-representation-unavailable",
        ),
        (
            Ty::UnresolvedLiteral(ast::ty::LiteralKind::Text),
            "literal-representation-unavailable",
        ),
        (unavailable_type(), "type-representation-unavailable"),
        (
            Ty::Literal(ast::ConstValue::Text("unmaterialized".to_owned())),
            "text-literal-requires-representation",
        ),
        (
            Ty::Sequence(vec![SequenceTypeElement {
                name: None,
                ty: Box::new(unavailable_type()),
            }]),
            "type-representation-unavailable",
        ),
    ] {
        let source = "value(): return\n";
        let mut typed = typed(source, 0);
        add_unrepresentable_parameter(&mut typed, "value", ty.clone());
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("invalid-representation.gin", source, &typed.span_table),
            &target(),
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.slug() == code),
            "{ty:?}: {diagnostics:?}"
        );
        assert!(
            !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.slug() == "codegen-invalid-module")
        );
        assert!(
            module.is_none(),
            "representation failure must not expose a fallback module"
        );
    }
}

#[test]
fn package_representation_flaws_remain_attached_to_the_failing_file() {
    let good_source = "good(): return\n";
    let bad_source = "bad(): return\n";
    let other_source = "other(): return\n";
    let good = typed(good_source, 0);
    let mut other = typed(other_source, 2);
    add_unrepresentable_parameter(
        &mut other,
        "other",
        Ty::Literal(ast::ConstValue::Text("unmaterialized".to_owned())),
    );
    let mut bad = typed(bad_source, 1);
    add_unrepresentable_parameter(&mut bad, "bad", unavailable_type());
    let context = NativeCompiler::create_context();
    for reverse in [false, true] {
        let good_input = ResolvedFileInput {
            program: good.resolved_program(),
            source_map: CodegenSourceMap::new("good.gin", good_source, &good.span_table),
        };
        let bad_input = ResolvedFileInput {
            program: bad.resolved_program(),
            source_map: CodegenSourceMap::new("bad.gin", bad_source, &bad.span_table),
        };
        let other_input = ResolvedFileInput {
            program: other.resolved_program(),
            source_map: CodegenSourceMap::new("other.gin", other_source, &other.span_table),
        };
        let files = if reverse {
            [other_input, bad_input, good_input]
        } else {
            [good_input, bad_input, other_input]
        };
        let output = CodegenContext::build_module_from_resolved_files_for_target(
            &context,
            &files,
            &target(),
        );
        assert!(output.module.is_none());
        assert_eq!(output.diagnostics.len(), 3);
        for file in output.diagnostics {
            if file.filename == "good.gin" {
                assert!(file.diagnostics.is_empty());
            } else {
                let (typed, name, code) = match file.filename.as_str() {
                    "bad.gin" => (&bad, "bad", "type-representation-unavailable"),
                    "other.gin" => (&other, "other", "text-literal-requires-representation"),
                    _ => panic!("unexpected diagnostic file"),
                };
                let bind = &typed.defs[&DefId::named(Intern::from_ref(name))];
                assert!(
                    file.diagnostics.iter().any(|diagnostic| {
                        diagnostic.code.slug() == code
                            && diagnostic.category == diagnostic::Category::Flaw
                            && diagnostic.span == typed.span_table.get(bind.name_span)
                    }),
                    "{:?}",
                    file.diagnostics
                );
            }
        }
    }
}

#[test]
fn successful_unit_module_has_no_codegen_flaws() {
    let source = "value(): return\n";
    let typed = typed(source, 0);
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("valid.gin", source, &typed.span_table),
        &target(),
    );
    assert!(module.is_some());
    assert!(diagnostics.is_empty());
}

#[test]
fn unavailable_named_return_type_cannot_be_erased_from_the_signature() {
    let source = "value(): return\n";
    let mut typed = typed(source, 0);
    let name = Intern::from_ref("Missing");
    let ty = Ty::Named {
        instance: ast::ty::NamedTypeInstance::new(ast::ty::TypeId { file: 0, name }),
        name,
    };
    let id = DefId::named(Intern::from_ref("value"));
    typed.defs.get_mut(&id).unwrap().return_type = ty.clone();
    typed.fn_return_types.insert(id, ty);
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("invalid-return.gin", source, &typed.span_table),
        &target(),
    );
    assert!(
        module.is_none(),
        "unavailable return storage must not become unit"
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code.slug() == "type-representation-unavailable"),
        "{diagnostics:?}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code.slug() == "codegen-invalid-module")
    );
}
