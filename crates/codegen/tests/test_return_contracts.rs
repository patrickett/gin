mod support;

use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use internment::Intern;
use support::typed;
use typecheck::{BindBody, DefId, TypedExprKind, TypedFileAst};

use test_fixtures::host_compile_target as target;

fn rejected(typed: &TypedFileAst, source: &str, code: &str) {
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("return_contract.gin", source, &typed.span_table),
        &target(),
    );
    assert!(module.is_none());
    let bind = &typed.defs[&DefId::named(Intern::new("value".to_owned()))];
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code.slug() == code
                && diagnostic.span == typed.span_table.get(bind.name_span)
        }),
        "{diagnostics:?}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "codegen-invalid-module")
    );
}

const PAIR: &str = "Byte is in 0 to 255\nByte has\n    finish(self):\n        return\nvalue() first Byte, second Byte: return 1, 2\n";

fn return_expr(typed: &TypedFileAst) -> typecheck::ExprId {
    match typed.defs[&DefId::named(Intern::new("value".to_owned()))].body {
        BindBody::Expr(expr)
        | BindBody::Body {
            ret: Some(expr), ..
        } => expr,
        _ => panic!("expected a return expression"),
    }
}

#[test]
fn missing_result_row_is_rejected_at_the_callable() {
    let mut typed = typed(PAIR);
    let bind = typed
        .defs
        .get_mut(&DefId::named(Intern::new("value".to_owned())))
        .unwrap();
    bind.body = BindBody::Body {
        exprs: Vec::new(),
        ret: None,
    };
    rejected(&typed, PAIR, "codegen-return-lowering-failed");
}

#[test]
fn wrong_result_count_is_rejected_before_emitting_return() {
    let mut typed = typed(PAIR);
    let expr = return_expr(&typed);
    let TypedExprKind::ResultRow(items) = &mut typed.exprs.kind[expr.as_usize()] else {
        panic!("expected result row");
    };
    items.pop().unwrap();
    rejected(&typed, PAIR, "codegen-return-type-mismatch");
}

#[test]
fn wrong_result_type_is_rejected_before_emitting_return() {
    let mut typed = typed(PAIR);
    let expr = return_expr(&typed);
    let TypedExprKind::ResultRow(items) = &typed.exprs.kind[expr.as_usize()] else {
        panic!("expected result row");
    };
    let first = items[0];
    typed.exprs.ty[first.as_usize()] = ast::ty::Ty::i64();
    rejected(&typed, PAIR, "codegen-return-type-mismatch");
}

#[test]
fn failed_statement_prevents_a_successful_unit_return() {
    let source = "value():\n    1\n    return\n";
    let mut typed = typed(source);
    let BindBody::Body { ref exprs, .. } =
        typed.defs[&DefId::named(Intern::new("value".to_owned()))].body
    else {
        panic!("expected block body");
    };
    let expr = exprs[0];
    typed.exprs.kind[expr.as_usize()] = TypedExprKind::Invalid;
    rejected(&typed, source, "codegen-return-lowering-failed");
}

#[test]
fn failed_sequence_binding_statement_prevents_a_successful_return() {
    let source = format!(
        "{}pair() first Byte, second Byte: return 1, 2\nvalue() first Byte, second Byte:\n    values() first Byte, second Byte:\n        1\n        return pair()\n    return values()\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let mut typed = typed(&source);
    let BindBody::Body { ref exprs, .. } =
        typed.defs[&DefId::named(Intern::from_ref("value"))].body
    else {
        panic!("expected block body");
    };
    let binding = exprs[0];
    assert!(matches!(
        typed.exprs.ty[binding.as_usize()],
        ast::Ty::Sequence(_)
    ));
    let TypedExprKind::Bind { ref stmts, .. } = typed.exprs.kind[binding.as_usize()] else {
        panic!("expected sequence binding");
    };
    let statement = stmts[0];
    typed.exprs.kind[statement.as_usize()] = TypedExprKind::Invalid;
    rejected(&typed, &source, "codegen-return-lowering-failed");
}

#[test]
#[allow(unsafe_code)]
fn loop_returns_initialize_every_result_on_taken_and_fallthrough_paths() {
    for loop_body in [
        "    while flag is 0 do\n        when flag is 0 then\n            return 11, 22\n",
        "    for index in 0 to 2\n        when flag is 0 then\n            return 11, 22\n",
        "    for element in [1, 2, 3]\n        when flag is 0 then\n            return 11, 22\n",
    ] {
        let source = format!(
            "{}Word is in 0 to 18446744073709551615\nWord has\n    finish(self):\n        return\nselect(flag Byte) first Word, second Word:\n{loop_body}    return 33, 44\nfirst(flag Byte) Word: return select(flag).first\nsecond(flag Byte) Word: return select(flag).second\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let typed = typed(&source);
        for (export, expected) in [("first", [11u64, 33]), ("second", [22u64, 44])] {
            let context = NativeCompiler::create_context();
            let (module, diagnostics) =
                CodegenContext::build_module_from_resolved_program_for_target(
                    &context,
                    typed.resolved_program(),
                    CodegenSourceMap::new("loop_returns.gin", &source, &typed.span_table),
                    &target(),
                );
            assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
            let engine = NativeEngine::new_with_export_abi(module.unwrap(), export).unwrap();
            for (mut flag, expected) in [0u8, 1].into_iter().zip(expected) {
                let mut result = 0u64;
                unsafe {
                    engine.invoke_packed(&mut [
                        (&mut flag as *mut u8).cast(),
                        (&mut result as *mut u64).cast(),
                    ])
                }
                .unwrap();
                assert_eq!(result, expected, "{source}, export={export}, flag={flag}");
            }
        }
    }
}
