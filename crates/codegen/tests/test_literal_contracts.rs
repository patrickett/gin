mod support;

use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use internment::Intern;
use support::typed;
use typecheck::{BindBody, DefId};

use test_fixtures::host_compile_target as target;

const PREFIX: &str = test_fixtures::preludes::BYTE_PRELUDE;

fn literal_engine(source: &str) -> NativeEngine {
    let typed = typed(source);
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("literal.gin", source, &typed.span_table),
        &target(),
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap()
}

#[test]
#[allow(unsafe_code)]
fn float_literals_preserve_native_bits() {
    let engine = literal_engine("select: 1.5\n");
    let mut result = 0f64;
    unsafe { engine.invoke_packed(&mut [(&mut result as *mut f64).cast()]) }.unwrap();
    assert_eq!(result.to_bits(), 1.5f64.to_bits());
}

#[test]
#[allow(unsafe_code)]
fn text_literals_preserve_utf8_bytes() {
    for text in ["abc", "hé", "🙂"] {
        let source = format!("select: '{text}'\n");
        let engine = literal_engine(&source);
        let mut result = vec![0u8; text.len()];
        unsafe { engine.invoke_packed(&mut [result.as_mut_ptr().cast()]) }.unwrap();
        assert_eq!(result, text.as_bytes());
    }
}

#[test]
#[allow(unsafe_code)]
fn text_literal_addresses_preserve_bytes_and_remain_stable() {
    for text in ["", "abc", "hé", "🙂", "a\0b"] {
        let literal = text.replace('\0', "\\0");
        let source = format!("{PREFIX}select() @Byte: return (@'{literal}') as @Byte\n");
        let engine = literal_engine(&source);
        let mut address = std::ptr::null_mut::<u8>();
        unsafe { engine.invoke_packed(&mut [(&mut address as *mut *mut u8).cast()]) }.unwrap();
        assert!(!address.is_null());
        let bytes = unsafe { std::slice::from_raw_parts(address, text.len()) };
        assert_eq!(bytes, text.as_bytes());
        let mut repeated = std::ptr::null_mut::<u8>();
        unsafe { engine.invoke_packed(&mut [(&mut repeated as *mut *mut u8).cast()]) }.unwrap();
        assert_eq!(address, repeated);
    }
}

#[test]
#[allow(unsafe_code)]
fn zero_after_a_record_argument_remains_zero() {
    let source = format!(
        "{PREFIX}identity(value Byte) Byte: return value\nBox has\n    value Byte\n    finish(self):\n        return\nconsume(value Box): return\nselect(value Box) Byte:\n    consume(value)\n    return 0\n"
    );
    for selected_storage in [false, true] {
        let mut typed = typed(&source);
        if selected_storage {
            let bind = &typed.defs[&DefId::named(Intern::from_ref("select"))];
            let returned = match bind.body {
                BindBody::Expr(id) | BindBody::Body { ret: Some(id), .. } => id,
                _ => panic!("return expression"),
            };
            typed.exprs.ty[returned.as_usize()] =
                typed.defs[&DefId::named(Intern::from_ref("identity"))].params[0]
                    .1
                    .clone();
        }
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("zero.gin", &source, &typed.span_table),
            &target(),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
        let mut value = 42u8;
        let mut result = 255u8;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut value as *mut u8).cast(),
                (&mut result as *mut u8).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, 0);
    }
}

#[test]
fn integer_literals_reject_missing_incompatible_and_out_of_range_facts() {
    for literal in ["0", "7"] {
        let source = format!(
            "{PREFIX}identity(value Byte) Byte: return value\nselect() Byte: return {literal}\n"
        );
        for (constant, expected) in [
            (None, "codegen-internal"),
            (
                Some(ast::ConstValue::Text("invalid".into())),
                "codegen-internal",
            ),
            (
                Some(ast::ConstValue::Int(256.into())),
                "codegen-integer-constant-domain",
            ),
            (
                Some(ast::ConstValue::Int(
                    "680564733841876926926749214863536422912".parse().unwrap(),
                )),
                "codegen-integer-constant-width",
            ),
        ] {
            let mut typed = typed(&source);
            let bind = &typed.defs[&DefId::named(Intern::from_ref("select"))];
            let returned = match bind.body {
                BindBody::Expr(id) | BindBody::Body { ret: Some(id), .. } => id,
                _ => panic!("return expression"),
            };
            typed.exprs.ty[returned.as_usize()] =
                typed.defs[&DefId::named(Intern::from_ref("identity"))].params[0]
                    .1
                    .clone();
            typed.exprs.facts.const_facts.const_value[returned.as_usize()] = constant;
            let context = NativeCompiler::create_context();
            let (module, diagnostics) =
                CodegenContext::build_module_from_resolved_program_for_target(
                    &context,
                    typed.resolved_program(),
                    CodegenSourceMap::new("literal_contract.gin", &source, &typed.span_table),
                    &target(),
                );
            assert!(
                module.is_none(),
                "invalid facts must not emit a constant: {literal}"
            );
            assert!(
                diagnostics.iter().any(|d| d.code.slug() == expected),
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
}
