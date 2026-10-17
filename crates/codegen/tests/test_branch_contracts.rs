mod support;

use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use internment::Intern;
use support::typed;
use typecheck::{PlaceId, PlaceVersionId, TypedExprKind};

use test_fixtures::host_compile_target as target;

fn source(when: bool, addressed: bool) -> String {
    let observe = if addressed {
        "    observe(value)\n"
    } else {
        ""
    };
    let body = if when {
        format!(
            "    value Byte: 1\n{observe}    choice Byte: when flag is\n        Flag.Left then value:: replace(value, 2)\n                  else value:: replace(value, 3)\n    return choice\n"
        )
    } else {
        format!(
            "    value Byte: 1\n{observe}    when flag is Flag.Left then\n        value:: replace(value, 2)\n    return value\n"
        )
    };
    format!(
        "{}Flag is Left or Right\nFlag has\n    finish(self):\n        return\nreplace(previous Byte, next Byte) Byte: return next\nobserve(value ref Byte) Byte: return deref value\nmain(flag Flag) Byte:\n{body}",
        test_fixtures::preludes::BYTE_PRELUDE
    )
}

#[test]
fn branch_joins_require_valid_incoming_bindings() {
    for when in [false, true] {
        let source = source(when, false);
        let original = typed(&source);
        let index = original
            .exprs
            .kind
            .iter()
            .position(|kind| match kind {
                TypedExprKind::When(body) => !body.place_joins.is_empty(),
                _ => false,
            })
            .expect("branch join");
        for corruption in ["binding", "version", "place"] {
            let mut typed = original.clone();
            let joins = match &mut typed.exprs.kind[index] {
                TypedExprKind::When(body) => &mut body.place_joins,
                _ => panic!("branch"),
            };
            let id = joins[0];
            let code = match corruption {
                "binding" => {
                    let place = typed.place_versions[id.0 as usize].place;
                    typed.places[place.0 as usize].name = Intern::from_ref("missing_incoming");
                    "codegen-branch-binding-missing"
                }
                "version" => {
                    joins[0] = PlaceVersionId(typed.place_versions.len().try_into().unwrap());
                    "codegen-branch-join-invalid"
                }
                "place" => {
                    typed.place_versions[id.0 as usize].place =
                        PlaceId(typed.places.len().try_into().unwrap());
                    "codegen-branch-join-invalid"
                }
                _ => unreachable!(),
            };
            let context = NativeCompiler::create_context();
            let (module, diagnostics) =
                CodegenContext::build_module_from_resolved_program_for_target(
                    &context,
                    typed.resolved_program(),
                    CodegenSourceMap::new("branch_contract.gin", &source, &typed.span_table),
                    &target(),
                );
            assert!(module.is_none());
            assert!(
                diagnostics.iter().any(|d| d.code.slug() == code),
                "{diagnostics:?}"
            );
            assert!(
                !diagnostics
                    .iter()
                    .any(|d| d.code.slug() == "codegen-invalid-module")
            );
        }
    }
}

#[test]
#[allow(unsafe_code)]
fn branch_joins_preserve_ssa_and_addressed_values() {
    for when in [false, true] {
        for addressed in [false, true] {
            let source = source(when, addressed);
            let typed = typed(&source);
            let context = NativeCompiler::create_context();
            let (module, diagnostics) =
                CodegenContext::build_module_from_resolved_program_for_target(
                    &context,
                    typed.resolved_program(),
                    CodegenSourceMap::new("branch_values.gin", &source, &typed.span_table),
                    &target(),
                );
            assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
            let module = module.unwrap();
            assert_eq!(
                module.as_operation().to_string().contains("llvm.alloca"),
                addressed
            );
            let engine = NativeEngine::new_with_export_abi(module, "main").unwrap();
            for mut flag in [0u8, 1] {
                let mut result = 0u8;
                unsafe {
                    engine.invoke_packed(&mut [
                        (&mut flag as *mut u8).cast(),
                        (&mut result as *mut u8).cast(),
                    ])
                }
                .unwrap();
                assert_eq!(
                    result,
                    if flag == 0 {
                        2
                    } else if when {
                        3
                    } else {
                        1
                    },
                    "{source}, flag={flag}"
                );
            }
        }
    }
}

#[test]
#[allow(unsafe_code)]
fn local_result_rows_honor_conditional_returns() {
    let source = format!(
        "{}Flag is Left or Right\nFlag has\n    finish(self):\n        return\npair(first Byte, second Byte) first Byte, second Byte: return first, second\nmain(flag Flag) Byte:\n    values() first Byte, second Byte:\n        when flag is Flag.Left then\n            return pair(2, 4)\n        return pair(1, 3)\n    return values().second\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let typed = typed(&source);
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("local_rows.gin", &source, &typed.span_table),
        &target(),
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let engine = NativeEngine::new_with_export_abi(module.unwrap(), "main").unwrap();
    for (mut flag, expected) in [(0u8, 4u8), (1, 3)] {
        let mut result = 0u8;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut flag as *mut u8).cast(),
                (&mut result as *mut u8).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, expected, "flag={flag}");
    }
}

#[test]
fn a_nonexhaustive_when_cannot_supply_a_result() {
    let source = "Byte is in 0 to 255\nByte.finish(self): return\nmain(flag Byte) Byte:\n    when flag = 0 then return 7\n    return 9\n";
    let mut typed = typed(source);
    let branch = typed
        .exprs
        .kind
        .iter()
        .position(|kind| matches!(kind, TypedExprKind::When(_)))
        .unwrap();
    let id = typecheck::DefId::named(Intern::from_ref("main"));
    typed.exprs.ty[branch] = typed.defs[&id].return_type.clone();
    typed.defs.get_mut(&id).unwrap().body =
        typecheck::BindBody::Expr(typecheck::ExprId(branch.try_into().unwrap()));
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("when-result.gin", source, &typed.span_table),
        &target(),
    );
    assert!(module.is_none());
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code.slug() == "codegen-when-result-unavailable"),
        "{diagnostics:?}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code.slug() == "codegen-invalid-module")
    );
}
