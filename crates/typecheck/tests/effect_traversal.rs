use ast::{Literal, SpanId, Ty};
use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::typed::DefinitionStage;
use typecheck::typed::{
    Availability, ConstFacts, ExprFacts, ExprId, OwnershipFacts, SemanticFacts, TypedExpr,
    TypedExprKind, TypedFileAst,
};
use typecheck::{BindBody, DefId, FileId};

fn push(typed: &mut TypedFileAst, kind: TypedExprKind, ty: Ty) -> ExprId {
    let id = ExprId(u32::try_from(typed.exprs.kind.len()).unwrap());
    typed.exprs.push(TypedExpr {
        kind,
        ty,
        span: SpanId::INVALID,
        facts: ExprFacts {
            const_facts: ConstFacts {
                const_value: None,
                const_object: None,
                sequence_literal: None,
                literal_materialization: None,
            },
            semantic_facts: SemanticFacts {
                integer_knowledge: None,
                result_evidence: None,
                projected_result_evidence: Default::default(),
                availability: Availability::Unknown,
                flaws: vec![],
            },
            ownership_facts: OwnershipFacts {
                references: Default::default(),
                reference_reads: Default::default(),
                target_group: None,
                result_target_groups: vec![],
                place: None,
                place_version: None,
            },
        },
    });
    id
}

#[test]
fn shared_expression_effect_traversal_is_bounded() {
    const CASE: &str = "GIN_TEST_EFFECT_TRAVERSAL_CASE";
    if let Ok(case) = std::env::var(CASE) {
        let parsed = "inspect:\n    return\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let mut typed = typecheck::transform::transform_file(parsed.ast, FileId(0));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let boundary = case == "boundary";
        let ty = if boundary {
            Ty::Opaque(Intern::from_ref("Target"))
        } else {
            Ty::bounded_int(0.into(), 0.into())
        };
        let mut root = push(
            &mut typed,
            if boundary {
                TypedExprKind::Target
            } else {
                TypedExprKind::Lit(Literal::Int(0.into()))
            },
            ty.clone(),
        );
        for level in 0..32 {
            root = push(
                &mut typed,
                TypedExprKind::Bind {
                    name: Intern::new(format!("shared{level}")),
                    stmts: vec![root],
                    body: root,
                    unassigned: false,
                },
                ty.clone(),
            );
        }
        let id = DefId::named(Intern::from_ref("inspect"));
        typed.defs.get_mut(&id).unwrap().body = BindBody::Body {
            exprs: vec![root],
            ret: None,
        };
        let _ = typecheck::transform::infer_package_full(
            std::slice::from_mut(&mut typed),
            &Default::default(),
        );
        assert_eq!(
            typed.defs[&id].definition_stage,
            if boundary {
                DefinitionStage::CompileTimeOnly
            } else {
                DefinitionStage::StagePolymorphic
            }
        );
        return;
    }
    for case in ["shared", "boundary"] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "shared_expression_effect_traversal_is_bounded",
                "--nocapture",
            ])
            .env(CASE, case)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn local_values_do_not_inherit_a_shadowed_functions_stage() {
    let prefix = format!(
        "{}selected := when #target.arch is\n    'x86_64' then (1 as Byte)\n    else (2 as Byte)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    for (source, expected) in [
        (
            "read(selected Byte) Byte: return selected\n",
            DefinitionStage::StagePolymorphic,
        ),
        (
            "read Byte:\n    selected: 1 as Byte\n    return selected\n",
            DefinitionStage::StagePolymorphic,
        ),
        ("read Byte: selected\n", DefinitionStage::CompileTimeOnly),
        (
            "read Byte:\n    for selected in 0 to selected\n        value: 0 as Byte\n        value.finish\n    return 0\n",
            DefinitionStage::CompileTimeOnly,
        ),
        (
            "read Byte:\n    for selected in 0 to 2\n        for other in 0 to selected\n            value: 0 as Byte\n            value.finish\n    return 0\n",
            DefinitionStage::StagePolymorphic,
        ),
        (
            "read Byte:\n    for selected in 0 to 2\n        value: selected as Byte\n        value.finish\n    return 0\n",
            DefinitionStage::StagePolymorphic,
        ),
        (
            "read Byte:\n    for selected in 0 to 2\n        value: selected as Byte\n        value.finish\n    return selected\n",
            DefinitionStage::CompileTimeOnly,
        ),
    ] {
        let parsed = format!("{prefix}{source}").parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{source}: {:?}",
            parsed.symptoms
        );
        let typed = typecheck::transform::transform_file(parsed.ast, FileId(0));
        assert!(
            typed.all_flaws().is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
        assert_eq!(
            typed.defs[&DefId::named(Intern::from_ref("selected"))].definition_stage,
            DefinitionStage::CompileTimeOnly
        );
        for kind in &typed.exprs.kind {
            let TypedExprKind::Loop(typecheck::typed::TypedLoop {
                kind:
                    typecheck::typed::TypedLoopKind::ForIn {
                        traversal:
                            typecheck::typed::SequenceTraversal::IntegerRange {
                                variable, stmts, ..
                            },
                        ..
                    },
                ..
            }) = kind
            else {
                continue;
            };
            let binding = *stmts.first().expect("range counter binding");
            let Some(TypedExprKind::Bind { body, .. }) = typed.exprs.kind_of(binding) else {
                panic!("range counter must have a binding")
            };
            assert!(
                matches!(typed.exprs.kind_of(*body), Some(TypedExprKind::FnCall { target, args: None, .. }) if target.0 == *variable)
            );
            assert!(typed.exprs.place_of(*body).flatten().is_some());
            assert!(typed.exprs.place_version_of(*body).flatten().is_some());
            assert!(matches!(
                typed.exprs.ty_of(*body),
                Some(Ty::AnonymousInteger { .. })
            ));
        }
        assert_eq!(
            typed.defs[&DefId::named(Intern::from_ref("read"))].definition_stage,
            expected,
            "{source}"
        );
    }
}

#[test]
fn range_counters_shadow_parameterized_functions() {
    let parsed = format!("{}selected(value Byte) Byte: return value\nread Byte:\n    for selected in 0 to 2\n        value: selected as Byte\n        value.finish\n    return 0\n", test_fixtures::preludes::BYTE_PRELUDE).parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = typecheck::transform::transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let read = &typed.defs[&DefId::named(Intern::from_ref("read"))];
    assert_eq!(read.definition_stage, DefinitionStage::StagePolymorphic);
    assert!(read.effects.writes.is_empty());
    assert!(!read.effects.unknown_memory);
}
