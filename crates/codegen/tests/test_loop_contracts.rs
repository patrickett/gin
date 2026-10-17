mod support;

use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use internment::Intern;
use support::typed;
use typecheck::{TypedExprKind, TypedFileAst};

use test_fixtures::host_compile_target as target;

const PRELUDE: &str = "Byte is in 0 to 255\nByte has\n    finish(self):\n        return\nreplace(previous Byte, next Byte) Byte: return next\n";

const LOOPS: [&str; 2] = [
    "main(flag Byte) Byte:\n    value Byte: 0\n    while flag is 0 do\n        value:: replace(value, 1)\n    return value\n",
    "main() Byte:\n    value Byte: 0\n    for index in 0 to 2\n        value:: replace(value, index as Byte)\n    return value\n",
];

fn rejected(typed: &TypedFileAst, source: &str, code: &str) {
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("loop_contract.gin", source, &typed.span_table),
        &target(),
    );
    assert!(module.is_none(), "invalid loop phi produced a module");
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

#[test]
fn loop_phis_reject_invalid_arena_references() {
    for body in LOOPS {
        let source = format!("{PRELUDE}{body}");
        let original = typed(&source);
        let index = original
            .exprs
            .kind
            .iter()
            .position(|kind| matches!(kind, TypedExprKind::Loop(_)))
            .unwrap();
        for missing_version in [false, true] {
            let mut typed = original.clone();
            let TypedExprKind::Loop(body) = &mut typed.exprs.kind[index] else {
                panic!("expected loop");
            };
            let phi = body.place_phis[0];
            if missing_version {
                body.place_phis[0] =
                    typecheck::PlaceVersionId(typed.place_versions.len().try_into().unwrap());
            } else {
                typed.place_versions[phi.0 as usize].place =
                    typecheck::PlaceId(typed.places.len().try_into().unwrap());
            }
            rejected(&typed, &source, "codegen-loop-phi-invalid");
        }
    }
}

#[test]
fn loop_phis_require_an_incoming_binding() {
    for body in LOOPS {
        let source = format!("{PRELUDE}{body}");
        let mut typed = typed(&source);
        let phi = typed
            .exprs
            .kind
            .iter()
            .find_map(|kind| match kind {
                TypedExprKind::Loop(body) => body.place_phis.first().copied(),
                _ => None,
            })
            .expect("loop-carried value");
        let place = typed.place_versions[phi.0 as usize].place;
        typed.places[place.0 as usize].name = Intern::from_ref("missing_incoming");
        rejected(&typed, &source, "codegen-loop-binding-missing");
    }
}

#[test]
#[allow(unsafe_code)]
fn counted_loops_preserve_ssa_and_addressed_bindings() {
    for addressed in [false, true] {
        let observe = if addressed {
            "    observe(value)\n"
        } else {
            ""
        };
        let source = format!(
            "{PRELUDE}observe(value ref Byte) Byte: return deref value\nmain() Byte:\n    value Byte: 0\n{observe}    for index in 0 to 2\n        value:: replace(value, index as Byte)\n    return value\n"
        );
        let typed = typed(&source);
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("loop_bindings.gin", &source, &typed.span_table),
            &target(),
        );
        assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
        let module = module.unwrap();
        assert_eq!(
            module.as_operation().to_string().contains("llvm.alloca"),
            addressed
        );
        let engine = NativeEngine::new_with_export_abi(module, "main").unwrap();
        let mut result = 0u8;
        unsafe { engine.invoke_packed(&mut [(&mut result as *mut u8).cast()]) }.unwrap();
        assert_eq!(result, 2, "addressed={addressed}");
    }
}

#[test]
#[allow(unsafe_code)]
fn range_counters_shadow_global_functions() {
    let source = format!(
        "{PRELUDE}index Byte: 99\nmain() Byte:\n    value Byte: 0\n    for index in 0 to 2\n        value:: replace(value, index as Byte)\n    return value\n"
    );
    let typed = typed(&source);
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("range_shadowing.gin", &source, &typed.span_table),
        &target(),
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let engine = NativeEngine::new_with_export_abi(module.unwrap(), "main").unwrap();
    let mut result = 0u8;
    unsafe { engine.invoke_packed(&mut [(&mut result as *mut u8).cast()]) }.unwrap();
    assert_eq!(result, 2);
}
