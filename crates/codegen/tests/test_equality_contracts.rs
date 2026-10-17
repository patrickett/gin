mod support;

use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use internment::Intern;
use support::typed;
use typecheck::{BindBody, DefId, TypedExprKind};

const TYPES: &str = test_fixtures::preludes::BYTE_PRELUDE;

use test_fixtures::host_compile_target as target;

#[test]
fn unresolved_equality_cannot_bypass_operator_resolution() {
    let source = format!(
        "{TYPES}callee(left Byte, right Byte) Byte: return left\nselect() Byte: return callee(7, 8)\n"
    );
    let mut typed = typed(&source);
    let bind = &typed.defs[&DefId::named(Intern::from_ref("select"))];
    let returned = match bind.body {
        BindBody::Expr(id) | BindBody::Body { ret: Some(id), .. } => id,
        _ => panic!("return expression"),
    };
    let TypedExprKind::FnCall {
        args: Some(args), ..
    } = &typed.exprs.kind[returned.as_usize()]
    else {
        panic!("call expression")
    };
    let [lhs, rhs] = args.as_slice() else {
        panic!("two arguments")
    };
    typed.exprs.kind[returned.as_usize()] = TypedExprKind::Binary {
        op: ast::BinOp::Equal,
        lhs: *lhs,
        rhs: *rhs,
    };
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("equality.gin", &source, &typed.span_table),
        &target(),
    );
    assert!(module.is_none(), "unresolved equality produced a module");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code.slug() == "codegen-internal"
                && d.message.contains("unresolved binary operator")),
        "{diagnostics:?}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code.slug() == "codegen-invalid-module"),
        "{diagnostics:?}"
    );
}

#[test]
#[allow(unsafe_code)]
fn resolved_intrinsic_and_declared_equality_preserve_native_results() {
    for declared in [false, true] {
        let body = if declared {
            "equal(left Byte, right Byte) Byte:\n    a is in 0 to 255: left\n    b is in 0 to 255: right\n    result: a = b\n    return when result then 1 else 0\nselect(left Byte, right Byte) Byte: return equal(left, right)\n".to_owned()
        } else {
            "select(left Byte, right Byte) Byte:\n    a is in 0 to 255: left\n    b is in 0 to 255: right\n    result: a = b\n    return when result then 1 else 0\n".to_owned()
        };
        let source = format!("{TYPES}{body}");
        let typed = typed(&source);
        assert!(!typed.exprs.kind.iter().any(|kind| matches!(
            kind,
            TypedExprKind::Binary {
                op: ast::BinOp::Equal,
                ..
            }
        )));
        if declared {
            assert!(typed.exprs.kind.iter().any(|kind| matches!(kind, TypedExprKind::FnCall { target, .. } if target.0.as_str() == "equal")));
        }
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("equality.gin", &source, &typed.span_table),
            &target(),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
        for (mut lhs, mut rhs) in [(0u8, 0u8), (7, 8), (128, 128), (255, 0), (255, 255)] {
            let mut result = 2u8;
            unsafe {
                engine.invoke_packed(&mut [
                    (&mut lhs as *mut u8).cast(),
                    (&mut rhs as *mut u8).cast(),
                    (&mut result as *mut u8).cast(),
                ])
            }
            .unwrap();
            assert_eq!(result, u8::from(lhs == rhs));
        }
    }
}
