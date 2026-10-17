use super::*;
use ast::ty::SequenceTypeElement;
use internment::Intern;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{DefId, FileId, TypedFileAst};

use test_fixtures::host_compile_target as target;

fn typed(source: &str) -> TypedFileAst {
    let parsed = parser::parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let mut scope = TransformCtx::new();
    scope.compile_target = target();
    let typed = transform(&parsed, FileId(0), &scope);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    typed
}

fn context<'a, 'c>(
    mlir: &'c melior::Context,
    typed: &'a TypedFileAst,
    source: &str,
) -> CodegenContext<'a, 'c> {
    CodegenContext::with_string_prefix(
        mlir,
        typed.resolved_program(),
        typecheck::layout::TargetLayout::from_compile_target(&target()).unwrap(),
        crate::CodegenSourceMap::new("cache.gin", source, &typed.span_table),
        "",
    )
}

fn definition<'a>(typed: &'a TypedFileAst, name: &str) -> &'a typecheck::TypedBind {
    &typed.defs[&DefId::named(Intern::new(name.to_owned()))]
}

#[test]
fn same_type_identity_in_separate_programs_uses_each_registry() {
    let source = |max| {
        format!(
            "Word is in 0 to {max}\nWord has\n    finish(self):\n        return\nidentity(value Word) Word: return value\n"
        )
    };
    let narrow_source = source(255);
    let wide_source = source(65535);
    let narrow = typed(&narrow_source);
    let wide = typed(&wide_source);
    let narrow_ty = &definition(&narrow, "identity").params[0].1;
    let wide_ty = &definition(&wide, "identity").params[0].1;
    assert_eq!(narrow_ty, wide_ty);
    let mlir = crate::NativeCompiler::create_context();
    let narrow_ctx = context(&mlir, &narrow, &narrow_source);
    let wide_ctx = context(&mlir, &wide, &wide_source);
    for _ in 0..3 {
        assert_eq!(narrow_ctx.ty_to_mlir(narrow_ty).to_string(), "i8");
        assert_eq!(wide_ctx.ty_to_mlir(wide_ty).to_string(), "i16");
    }
    assert!(narrow_ctx.drain_symptoms().is_empty());
    assert!(wide_ctx.drain_symptoms().is_empty());
}

#[test]
fn same_record_identity_in_separate_programs_preserves_empty_abi() {
    let empty_source =
        "Item has\n    finish(self):\n        return\nidentity(value Item) Item: return value\n";
    let stored_source = format!(
        "{}Item has\n    field Byte\n    finish(self):\n        return\nidentity(value Item) Item: return value\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let empty = typed(empty_source);
    let stored = typed(&stored_source);
    let empty_ty = &definition(&empty, "identity").params[0].1;
    let stored_ty = &definition(&stored, "identity").params[0].1;
    assert_eq!(empty_ty, stored_ty);
    let mlir = crate::NativeCompiler::create_context();
    let empty_ctx = context(&mlir, &empty, empty_source);
    let stored_ctx = context(&mlir, &stored, &stored_source);
    for _ in 0..3 {
        assert!(empty_ctx.is_empty_product(empty_ty));
        assert!(!stored_ctx.is_empty_product(stored_ty));
        assert!(empty_ctx.result_types_to_mlir(empty_ty).is_empty());
        assert_eq!(stored_ctx.result_types_to_mlir(stored_ty).len(), 1);
    }
    assert!(empty_ctx.drain_symptoms().is_empty());
    assert!(stored_ctx.drain_symptoms().is_empty());
}

#[test]
fn generic_arguments_distinguish_cached_array_lengths() {
    let source = format!(
        "{}#fixed_array(element, count)\nBuffer(element, count BigInt) has\n    finish(self):\n        return\nzero(value Buffer(Byte, 0)) Buffer(Byte, 0): return value\ntwo(value Buffer(Byte, 2)) Buffer(Byte, 2): return value\nthree(value Buffer(Byte, 3)) Buffer(Byte, 3): return value\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let typed = typed(&source);
    let mlir = crate::NativeCompiler::create_context();
    let ctx = context(&mlir, &typed, &source);
    for _ in 0..3 {
        for (name, length) in [("zero", 0), ("two", 2), ("three", 3)] {
            let ty = &definition(&typed, name).params[0].1;
            assert_eq!(
                ctx.ty_to_mlir(ty).to_string(),
                format!("!llvm.array<{length} x i8>")
            );
            assert_eq!(ctx.is_empty_product(ty), length == 0);
        }
    }
    assert!(ctx.drain_symptoms().is_empty());
}

#[test]
fn literal_only_mappings_emit_at_each_call_site() {
    let source = "first: 1\nsecond: 2\n";
    let typed = typed(source);
    let mlir = crate::NativeCompiler::create_context();
    let ctx = context(&mlir, &typed, source);
    let domain = Ty::bounded_int(0.into(), 1.into())
        .anonymous_integer_validity()
        .unwrap()
        .domain()
        .clone();
    for value in [
        ast::ConstValue::Int(1.into()),
        ast::ConstValue::Float(ast::HashFloat(1.5)),
        ast::ConstValue::Domain(std::sync::Arc::new(domain)),
        ast::ConstValue::Tag {
            name: Intern::from_ref("Atom"),
            qual_path: None,
            args: [].into(),
        },
        ast::ConstValue::ResultAlternative {
            owner: ast::ResultFamilyOwner::LocalCallable(ast::BinderId::new(
                0,
                ast::BinderOwner::Expression(0),
            )),
            label: Intern::from_ref("True"),
        },
        ast::ConstValue::Record { fields: [].into() },
        ast::ConstValue::ResultRow([].into()),
        ast::ConstValue::FixedArray([].into()),
        ast::ConstValue::Sequence([].into()),
        ast::ConstValue::SequenceRepeat {
            seed: Box::new(ast::ConstValue::Int(0.into())),
            count: Box::new(ast::NormalExpr::from(0i128)),
        },
    ] {
        let ty = Ty::Sequence(vec![SequenceTypeElement {
            name: None,
            ty: Box::new(Ty::Literal(value)),
        }]);
        for name in ["first", "second", "first"] {
            let span = definition(&typed, name).name_span;
            ctx.current_span.set(span);
            ctx.ty_to_mlir(&ty);
            let diagnostics = ctx.drain_symptoms();
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            assert_eq!(
                diagnostics[0].code.slug().as_str(),
                "literal-representation-unavailable"
            );
            assert_eq!(diagnostics[0].span, typed.span_table.get(span));
        }
    }
}

#[test]
fn unresolved_literal_mappings_emit_at_each_call_site() {
    let source = "first: 1\nsecond: 2\n";
    let typed = typed(source);
    let mlir = crate::NativeCompiler::create_context();
    let ctx = context(&mlir, &typed, source);
    for kind in [ast::ty::LiteralKind::Integer, ast::ty::LiteralKind::Text] {
        let ty = Ty::Sequence(vec![SequenceTypeElement {
            name: None,
            ty: Box::new(Ty::UnresolvedLiteral(kind)),
        }]);
        for name in ["first", "second", "first"] {
            let span = definition(&typed, name).name_span;
            ctx.current_span.set(span);
            ctx.ty_to_mlir(&ty);
            let diagnostics = ctx.drain_symptoms();
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            assert_eq!(
                diagnostics[0].code.slug().as_str(),
                "literal-representation-unavailable"
            );
            assert_eq!(diagnostics[0].span, typed.span_table.get(span));
        }
    }
}

#[test]
fn failed_nested_mappings_emit_at_every_call_site_even_after_draining() {
    let source = "first: 1\nsecond: 2\n";
    let typed = typed(source);
    let mlir = crate::NativeCompiler::create_context();
    let ctx = context(&mlir, &typed, source);
    let ty = Ty::Sequence(vec![SequenceTypeElement {
        name: None,
        ty: Box::new(Ty::Opaque(Intern::new("Missing".to_owned()))),
    }]);
    for name in ["first", "second", "first"] {
        let span = definition(&typed, name).name_span;
        ctx.current_span.set(span);
        ctx.ty_to_mlir(&ty);
        let diagnostics = ctx.drain_symptoms();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(
            diagnostics[0].code.slug().as_str(),
            "unsupported-type-representation"
        );
        assert_eq!(diagnostics[0].span, typed.span_table.get(span));
    }
}

#[test]
fn empty_products_and_nested_scalars_remain_distinct() {
    let source = "value: 1\n";
    let typed = typed(source);
    let mlir = crate::NativeCompiler::create_context();
    let ctx = context(&mlir, &typed, source);
    let sequence = Ty::Sequence(vec![SequenceTypeElement {
        name: None,
        ty: Box::new(Ty::i64()),
    }]);
    for _ in 0..3 {
        assert!(ctx.is_empty_product(&Ty::Unit));
        assert!(ctx.is_empty_product(&Ty::Sequence(Vec::new())));
        assert!(!ctx.is_empty_product(&sequence));
        assert_eq!(ctx.result_types_to_mlir(&sequence)[0].to_string(), "i64");
        assert!(ctx.result_types_to_mlir(&Ty::Unit).is_empty());
    }
    assert!(ctx.drain_symptoms().is_empty());
}

#[test]
fn unavailable_named_representation_is_not_an_empty_result() {
    let source = "first: 1\nsecond: 2\n";
    let typed = typed(source);
    let mlir = crate::NativeCompiler::create_context();
    let ctx = context(&mlir, &typed, source);
    let name = Intern::from_ref("Missing");
    let ty = Ty::Named {
        instance: ast::ty::NamedTypeInstance::new(ast::ty::TypeId { file: 0, name }),
        name,
    };
    for callable in ["first", "second", "first"] {
        let span = definition(&typed, callable).name_span;
        ctx.current_span.set(span);
        assert!(!ctx.is_empty_product(&ty));
        assert_eq!(ctx.result_types_to_mlir(&ty).len(), 1);
        let diagnostics = ctx.drain_symptoms();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(
            diagnostics[0].code.slug().as_str(),
            "type-representation-unavailable"
        );
        assert_eq!(diagnostics[0].span, typed.span_table.get(span));
    }
}
