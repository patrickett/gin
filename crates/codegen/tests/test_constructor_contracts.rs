mod support;

use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use support::typed;
use typecheck::TypedExprKind;

use test_fixtures::host_compile_target as target;

const PREFIX: &str = "Byte is in 0 to 255\nByte has\n    finish(self):\n        return\nPair has\n    first Byte\n    second Byte\n    finish(self):\n        return\n";

#[test]
fn failed_constructor_arguments_cannot_be_discarded() {
    let source = format!(
        "{PREFIX}sink(first Byte, second Byte):\n    Pair(first: first, second: second)\n    return\n"
    );
    let mut typed = typed(&source);
    let argument = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            TypedExprKind::TagCall {
                args: Some(args), ..
            } if args.len() == 2 => Some(args[0]),
            _ => None,
        })
        .expect("record constructor");
    typed.exprs.kind[argument.as_usize()] = TypedExprKind::Invalid;
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("constructor.gin", &source, &typed.span_table),
        &target(),
    );
    assert!(
        module.is_none(),
        "failed child expression must fail its constructor"
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code.slug() == "codegen-return-lowering-failed"),
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
fn named_record_arguments_use_declared_field_positions() {
    let source = format!(
        "{PREFIX}select() Byte:\n    pair: Pair(second: 22, first: 11)\n    return pair.first\n"
    );
    let typed = typed(&source);
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("constructor.gin", &source, &typed.span_table),
        &target(),
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
    let mut result = 0u8;
    unsafe { engine.invoke_packed(&mut [(&mut result as *mut u8).cast()]) }.unwrap();
    assert_eq!(result, 11);
}

#[test]
fn record_constructors_reject_inconsistent_field_metadata() {
    for case in 0..5 {
        let source = format!(
            "{PREFIX}sink(first Byte, second Byte):\n    Pair(first: first, second: second)\n    return\n"
        );
        let mut typed = typed(&source);
        let constructor = typed.exprs.kind.iter().position(|kind| matches!(kind, TypedExprKind::TagCall { args: Some(args), .. } if args.len() == 2)).unwrap();
        let TypedExprKind::TagCall {
            args: Some(args),
            field_names,
            ..
        } = &mut typed.exprs.kind[constructor]
        else {
            unreachable!()
        };
        match case {
            0 => {
                args.pop();
            }
            1 => {
                args.push(args[0]);
            }
            2 => {
                field_names.pop();
            }
            3 => {
                field_names[0] = internment::Intern::from_ref("missing");
            }
            4 => {
                field_names[0] = field_names[1];
            }
            _ => unreachable!(),
        }
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("constructor.gin", &source, &typed.span_table),
            &target(),
        );
        assert!(module.is_none(), "case={case}");
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-constructor-invalid"),
            "case={case}: {diagnostics:?}"
        );
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-invalid-module"),
            "case={case}: {diagnostics:?}"
        );
    }
}

#[test]
fn failed_list_elements_cannot_be_discarded() {
    for broken in 0..2 {
        let source = format!(
            "{PREFIX}sink(first Byte, second Byte):\n    Pair(first: first, second: second)\n    return\n"
        );
        let mut typed = typed(&source);
        let (constructor, args) = typed
            .exprs
            .kind
            .iter()
            .enumerate()
            .find_map(|(i, kind)| match kind {
                TypedExprKind::TagCall {
                    args: Some(args), ..
                } if args.len() == 2 => Some((i, args.clone())),
                _ => None,
            })
            .unwrap();
        typed.exprs.ty[constructor] = typed.exprs.ty[args[0].as_usize()].clone();
        typed.exprs.kind[constructor] = TypedExprKind::List(args.clone());
        typed.exprs.kind[args[broken].as_usize()] = TypedExprKind::Invalid;
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("list.gin", &source, &typed.span_table),
            &target(),
        );
        assert!(module.is_none(), "element={broken}");
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-return-lowering-failed"),
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
fn reordered_fields_preserve_argument_evaluation_order() {
    let fixture = test_fixtures::emission::increment_source(&test_fixtures::host_target_triple())
        .replace(
            "Word has\n",
            "Word has\n    copy(self ref) Self: return deref self\n",
        );
    let source = format!(
        "{fixture}\nPair has\n    first Word\n    second Word\n    finish(self):\n        return\nreplace(previous Word, next Word) Word: return next\nbump(value mut Word) Word:\n    value:: replace(deref value, increment(Word.copy(value)))\n    return Word.copy(value)\nselect(state mut Word) Word:\n    pair: Pair(second: bump(state), first: bump(state))\n    return pair.first\n"
    );
    let typed = typed(&source);
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("ordered-constructor.gin", &source, &typed.span_table),
        &target(),
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
    let mut state = 41u64;
    let mut pointer = &mut state as *mut u64;
    let mut result = 0u64;
    unsafe {
        engine.invoke_packed(&mut [
            (&mut pointer as *mut *mut u64).cast(),
            (&mut result as *mut u64).cast(),
        ])
    }
    .unwrap();
    assert_eq!(result, 43);
    assert_eq!(state, 43);
}

#[test]
#[allow(unsafe_code)]
fn reordered_fields_preserve_heterogeneous_storage_types() {
    let source = format!(
        "{PREFIX}Word is in 0 to 18446744073709551615\nWord has\n    finish(self):\n        return\nPadded has\n    first Byte\n    second Word\n    finish(self):\n        return\nmake(first Byte, second Word) Padded: return Padded(second: second, first: first)\nselect() Word: return make(11, 4294967296).second\n"
    );
    let typed = typed(&source);
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("padded-constructor.gin", &source, &typed.span_table),
        &target(),
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
    let mut result = 0u64;
    unsafe { engine.invoke_packed(&mut [(&mut result as *mut u64).cast()]) }.unwrap();
    assert_eq!(result, 4294967296);
}

#[test]
fn sum_constructor_variant_and_discriminant_must_agree() {
    let source = format!(
        "{PREFIX}Maybe(element) is Some(element) or None\nMaybe(element) has\n    finish(self):\n        return\nsink(value Byte):\n    Maybe.Some(value)\n    return\n"
    );
    for wrong_discriminant in [1, usize::MAX] {
        let mut typed = typed(&source);
        let constructor = typed.exprs.kind.iter_mut().find(|kind| matches!(kind, TypedExprKind::TagCall { args: Some(args), .. } if args.len() == 1)).unwrap();
        let TypedExprKind::TagCall { discriminant, .. } = constructor else {
            unreachable!()
        };
        *discriminant = wrong_discriminant;
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("sum-constructor.gin", &source, &typed.span_table),
            &target(),
        );
        assert!(module.is_none());
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-constructor-invalid"),
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
fn integer_fields_use_declared_width_and_source_signedness() {
    for (declaration, bounds, input, expected) in [
        ("Byte", "0 to 127", "42", 42i8),
        ("Signed", "-64 to 63", "-42", -42i8),
    ] {
        let source = format!(
            "{PREFIX}Signed is in -128 to 127\nSigned has\n    finish(self):\n        return\nBox has\n    value {declaration}\n    finish(self):\n        return\nmake(value is in {bounds}) Box: return Box(value: value)\nselect() {declaration}: return make({input}).value\n"
        );
        let typed = typed(&source);
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("field-width.gin", &source, &typed.span_table),
            &target(),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
        let mut result = 0i8;
        unsafe { engine.invoke_packed(&mut [(&mut result as *mut i8).cast()]) }.unwrap();
        assert_eq!(result, expected);
    }
}

#[test]
fn constructor_values_must_have_compatible_storage_types() {
    let source = format!(
        "{PREFIX}unit(): return\nsink(first Byte, second Byte):\n    Pair(first: first, second: second)\n    return\n"
    );
    let mut typed = typed(&source);
    let arg = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            TypedExprKind::TagCall {
                args: Some(args), ..
            } if args.len() == 2 => Some(args[0]),
            _ => None,
        })
        .unwrap();
    typed.exprs.ty[arg.as_usize()] = ast::Ty::Unit;
    typed.exprs.kind[arg.as_usize()] = TypedExprKind::FnCall {
        target: typecheck::DefId::named(internment::Intern::from_ref("unit")),
        args: Some(Vec::new()),
        substituted_ty: None,
    };
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("field-type.gin", &source, &typed.span_table),
        &target(),
    );
    assert!(module.is_none());
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code.slug() == "codegen-constructor-invalid"),
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
fn literal_unions_encode_members_instead_of_literal_bytes() {
    for (literal, expected) in [("debug", 0u8), ("info", 1), ("warn", 2), ("error", 3)] {
        for body in [
            format!("    return '{literal}' as LogLevel\n"),
            format!("    level LogLevel: '{literal}' as LogLevel\n    return level\n"),
        ] {
            let source = format!(
                "LogLevel is 'debug' or 'info' or 'warn' or 'error'\nselect() LogLevel:\n{body}"
            );
            let typed = typed(&source);
            let context = NativeCompiler::create_context();
            let (module, diagnostics) =
                CodegenContext::build_module_from_resolved_program_for_target(
                    &context,
                    typed.resolved_program(),
                    CodegenSourceMap::new("literal-union.gin", &source, &typed.span_table),
                    &target(),
                );
            assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
            let engine = NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap();
            let mut result = 0u8;
            unsafe { engine.invoke_packed(&mut [(&mut result as *mut u8).cast()]) }.unwrap();
            assert_eq!(result & 3, expected, "{source}");
        }
    }
}

#[test]
fn literal_union_values_must_belong_to_the_selected_union() {
    let source = "LogLevel is 'debug' or 'info' or 'warn' or 'error'\nselect() LogLevel:\n    level LogLevel: 'warn' as LogLevel\n    return level\n";
    let mut typed = typed(source);
    let literal = typed
        .exprs
        .kind
        .iter()
        .position(
            |kind| matches!(kind, TypedExprKind::Lit(ast::Literal::String(text)) if text == "warn"),
        )
        .unwrap();
    typed.exprs.facts.const_facts.const_value[literal] =
        Some(ast::ConstValue::Text("absent".into()));
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("literal-union.gin", source, &typed.span_table),
        &target(),
    );
    assert!(module.is_none());
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code.slug() == "codegen-literal-union-invalid"),
        "{diagnostics:?}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code.slug() == "codegen-invalid-module"),
        "{diagnostics:?}"
    );
}
