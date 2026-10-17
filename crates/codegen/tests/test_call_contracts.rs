mod support;

use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use internment::Intern;
use support::typed;
use typecheck::{BindBody, DefId, TypedExprKind};

const TYPES: &str = test_fixtures::preludes::BYTE_PRELUDE;

use test_fixtures::host_compile_target as target;

#[test]
fn missing_call_targets_fail_before_backend_verification() {
    for body in [
        "callee() Byte: return 7\nselect() Byte: return callee()\n",
        "callee(): return\nselect(): return callee()\n",
        "callee() first Byte, second Byte: return 1, 2\nselect() first Byte, second Byte: return callee()\n",
        "callee(value Byte) Byte: return value\nselect() Byte: return callee(7)\n",
    ] {
        let source = format!("{TYPES}{body}");
        let mut typed = typed(&source);
        let bind = &typed.defs[&DefId::named(Intern::from_ref("select"))];
        let call = match bind.body {
            BindBody::Expr(id) | BindBody::Body { ret: Some(id), .. } => id,
            _ => panic!("call expression"),
        };
        let TypedExprKind::FnCall {
            target: call_target,
            ..
        } = &mut typed.exprs.kind[call.as_usize()]
        else {
            panic!("call expression")
        };
        *call_target = DefId::named(Intern::from_ref("absent"));
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("call.gin", &source, &typed.span_table),
            &target(),
        );
        assert!(module.is_none(), "missing target produced a module");
        assert!(
            diagnostics
                .iter()
                .any(|d| { d.code.slug() == "codegen-internal" && d.message.contains("absent") }),
            "{diagnostics:?}"
        );
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-invalid-module"),
            "{diagnostics:?}"
        );
    }
}

#[test]
#[allow(unsafe_code)]
fn known_calls_and_local_values_preserve_results() {
    for body in [
        "callee() Byte: return 7\nselect() Byte: return callee()\n",
        "callee(value Byte) Byte: return value\nselect() Byte: return callee(7)\n",
        "select() Byte:\n    value Byte: 7\n    return value\n",
    ] {
        let source = format!("{TYPES}{body}");
        let typed = typed(&source);
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("call.gin", &source, &typed.span_table),
            &target(),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
        let mut result = 0u8;
        unsafe { engine.invoke_packed(&mut [(&mut result as *mut u8).cast()]) }.unwrap();
        assert_eq!(result, 7);
    }
}
