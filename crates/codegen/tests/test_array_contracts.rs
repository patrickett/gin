use ast::{ConstValue, NormalExpr, Ty};
use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler};
use internment::Intern;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, FileId, TypedExprKind};

enum ArrayForm {
    Literal,
    List,
    Repeat,
}

fn check(source: &str, length: u64, form: ArrayForm, expected: &str) {
    let parsed = parser::parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    let mut scope = TransformCtx::new();
    scope.compile_target = target.clone();
    let mut typed = transform(&parsed, FileId(0), &scope);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let bind = &typed.defs[&DefId::named(Intern::new("make".to_owned()))];
    let expr = match bind.body {
        BindBody::Expr(expr)
        | BindBody::Body {
            ret: Some(expr), ..
        } => expr,
        _ => panic!("expected return expression"),
    };
    let Ty::Array { size, .. } = &mut typed.exprs.ty[expr.as_usize()] else {
        panic!("expected structural array");
    };
    *size = NormalExpr::Value(ConstValue::Int(length.into()));
    if matches!(form, ArrayForm::List) {
        let TypedExprKind::ArrayLit(items) = &typed.exprs.kind[expr.as_usize()] else {
            panic!("expected array literal");
        };
        typed.exprs.kind[expr.as_usize()] = TypedExprKind::List(items.clone());
    } else if matches!(form, ArrayForm::Repeat) {
        let TypedExprKind::FixedArrayAlloc { size, .. } = &mut typed.exprs.kind[expr.as_usize()]
        else {
            panic!("expected repeated array");
        };
        *size = NormalExpr::Value(ConstValue::Int(length.into()));
    } else {
        assert!(matches!(
            typed.exprs.kind[expr.as_usize()],
            TypedExprKind::ArrayLit(_)
        ));
    }
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("array_contract.gin", source, &typed.span_table),
        &target,
    );
    assert!(module.is_none());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == expected),
        "{diagnostics:?}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "codegen-invalid-module")
    );
}

const LITERAL: &str = "Byte is in 0 to 255\nByte has\n    finish(self):\n        return\nmake(first Byte, second Byte, third Byte) [Byte; 3]: return [first, second, third]\n";

#[test]
fn array_literals_reject_missing_and_extra_elements() {
    for length in [0, 2, 4] {
        check(
            LITERAL,
            length,
            ArrayForm::Literal,
            "codegen-array-length-mismatch",
        );
    }
}

#[test]
fn list_lowering_rejects_missing_and_extra_array_elements() {
    for length in [0, 2, 4] {
        check(
            LITERAL,
            length,
            ArrayForm::List,
            "codegen-array-length-mismatch",
        );
    }
}

#[test]
fn repeated_arrays_reject_unrepresentable_dimensions_before_expansion() {
    let source = format!(
        "{}make(value Byte) [Byte; 3]: return [value; 3]\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    check(
        &source,
        u64::from(u32::MAX) + 1,
        ArrayForm::Repeat,
        "array-layout-overflow",
    );
}

#[test]
fn named_array_constructor_cannot_replace_failed_seed_with_zero() {
    for stale_constant in [None, Some(ConstValue::Int(9.into()))] {
        let source = format!(
            "{}#fixed_array(element, count)\nBuffer(element, count BigInt) has\n    finish(self):\n        return\nmake(value Byte) Buffer(Byte, 3): return [value; 3]\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let parsed = parser::parse_from_str(&source);
        assert!(parsed.parse_warnings.is_empty());
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let mut typed = transform(&parsed, FileId(0), &scope);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let bind = &typed.defs[&DefId::named(Intern::from_ref("make"))];
        let returned = match bind.body {
            BindBody::Expr(id) | BindBody::Body { ret: Some(id), .. } => id,
            _ => panic!("return expression"),
        };
        let seed = match &typed.exprs.kind[returned.as_usize()] {
            TypedExprKind::ArrayLit(items) => items[0],
            TypedExprKind::FixedArrayAlloc { init, .. } => *init,
            kind => panic!("expected repeated array: {kind:?}"),
        };
        typed.exprs.kind[seed.as_usize()] = TypedExprKind::Invalid;
        typed.exprs.facts.const_facts.const_value[seed.as_usize()] = stale_constant;
        typed.exprs.kind[returned.as_usize()] = TypedExprKind::TagCall {
            variant_id: typecheck::VariantId {
                union: typecheck::TagId(Intern::from_ref("Buffer")),
                name: Intern::from_ref("Buffer"),
            },
            discriminant: 0,
            args: Some(vec![seed]),
            field_names: Vec::new(),
        };
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("seed.gin", &source, &typed.span_table),
            &target,
        );
        assert!(
            module.is_none(),
            "failed seed must not become an initialized zero array"
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
}

#[test]
fn array_constructors_reject_missing_and_extra_seed_arguments() {
    for count in [0, 2] {
        let source = format!(
            "{}#fixed_array(element, count)\nBuffer(element, count BigInt) has\n    finish(self):\n        return\nmake(value Byte) Buffer(Byte, 3): return [value; 3]\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let parsed = parser::parse_from_str(&source);
        assert!(parsed.parse_warnings.is_empty());
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let mut typed = transform(&parsed, FileId(0), &scope);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let bind = &typed.defs[&DefId::named(Intern::from_ref("make"))];
        let returned = match bind.body {
            BindBody::Expr(id) | BindBody::Body { ret: Some(id), .. } => id,
            _ => panic!("return expression"),
        };
        let seed = match &typed.exprs.kind[returned.as_usize()] {
            TypedExprKind::ArrayLit(items) => items[0],
            TypedExprKind::FixedArrayAlloc { init, .. } => *init,
            _ => panic!("repeated array"),
        };
        typed.exprs.kind[returned.as_usize()] = TypedExprKind::TagCall {
            variant_id: typecheck::VariantId {
                union: typecheck::TagId(Intern::from_ref("Buffer")),
                name: Intern::from_ref("Buffer"),
            },
            discriminant: 0,
            args: Some(vec![seed; count]),
            field_names: Vec::new(),
        };
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("seed.gin", &source, &typed.span_table),
            &target,
        );
        assert!(module.is_none());
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-array-constructor-arity"),
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
fn fixed_repeat_counts_must_match_selected_storage() {
    let source = format!(
        "{}make(value Byte) [Byte; 3]: return [value; 3]\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let parsed = parser::parse_from_str(&source);
    assert!(parsed.parse_warnings.is_empty());
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    let mut scope = TransformCtx::new();
    scope.compile_target = target.clone();
    let original = transform(&parsed, FileId(0), &scope);
    assert!(original.all_flaws().is_empty());
    let returned = match original.defs[&DefId::named(Intern::from_ref("make"))].body {
        BindBody::Expr(id) | BindBody::Body { ret: Some(id), .. } => id,
        _ => panic!("return expression"),
    };
    for size in [
        ast::NormalExpr::from(2i128),
        ast::NormalExpr::from(4i128),
        ast::NormalExpr::Var(Intern::from_ref("runtime")),
    ] {
        let mut typed = original.clone();
        let TypedExprKind::FixedArrayAlloc {
            size: selected_size,
            ..
        } = &mut typed.exprs.kind[returned.as_usize()]
        else {
            panic!("repeated array")
        };
        *selected_size = size;
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("repeat_count.gin", &source, &typed.span_table),
            &target,
        );
        assert!(module.is_none(), "invalid repeat count produced a module");
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-array-length-mismatch"),
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
fn array_elements_must_match_selected_storage() {
    for form in [ArrayForm::Literal, ArrayForm::List, ArrayForm::Repeat] {
        let source = format!(
            "{}make(value Byte) [Byte; 3]: return [value; 3]\nfloating: 1.5\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let parsed = parser::parse_from_str(&source);
        assert!(parsed.parse_warnings.is_empty());
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let mut typed = transform(&parsed, FileId(0), &scope);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let bind = &typed.defs[&DefId::named(Intern::from_ref("make"))];
        let returned = match bind.body {
            BindBody::Expr(id) | BindBody::Body { ret: Some(id), .. } => id,
            _ => panic!("return expression"),
        };
        let seed = match &typed.exprs.kind[returned.as_usize()] {
            TypedExprKind::ArrayLit(items) => items[0],
            TypedExprKind::FixedArrayAlloc { init, .. } => *init,
            _ => panic!("repeated array"),
        };
        typed.exprs.kind[seed.as_usize()] =
            TypedExprKind::Lit(ast::Literal::Float(ast::HashFloat(1.5)));
        let floating = &typed.defs[&DefId::named(Intern::from_ref("floating"))];
        typed.exprs.ty[seed.as_usize()] = floating.return_type.clone();
        typed.exprs.facts.const_facts.const_value[seed.as_usize()] =
            Some(ast::ConstValue::Float(ast::HashFloat(1.5)));
        typed.exprs.kind[returned.as_usize()] = match form {
            ArrayForm::Literal => TypedExprKind::ArrayLit(vec![seed; 3]),
            ArrayForm::List => TypedExprKind::List(vec![seed; 3]),
            ArrayForm::Repeat => TypedExprKind::FixedArrayAlloc {
                init: seed,
                size: ast::NormalExpr::from(3i128),
            },
        };
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("element.gin", &source, &typed.span_table),
            &target,
        );
        assert!(module.is_none());
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-array-element-type-mismatch"),
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
