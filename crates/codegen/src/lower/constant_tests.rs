use super::*;
use internment::Intern;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{DefId, FileId};

#[test]
fn string_byte_lengths_respect_the_llvm_array_limit() {
    let source = "select: 1.5\n";
    let parsed = parser::parse_from_str(source);
    assert!(parsed.parse_warnings.is_empty());
    let typed = transform(&parsed, FileId(0), &TransformCtx::new());
    assert!(typed.all_flaws().is_empty());
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    let mlir = crate::NativeCompiler::create_context();
    let context = CodegenContext::with_string_prefix(
        &mlir,
        typed.resolved_program(),
        typecheck::layout::TargetLayout::from_compile_target(&target).unwrap(),
        CodegenSourceMap::new("string_length.gin", source, &typed.span_table),
        "",
    );
    for length in [0, 1, u32::MAX] {
        assert_eq!(
            context.checked_string_byte_length(length as usize),
            Some(length)
        );
    }
    assert!(context.drain_symptoms().is_empty());
    if let Some(length) = (u32::MAX as usize).checked_add(1) {
        assert_eq!(context.checked_string_byte_length(length), None);
        assert!(
            context
                .drain_symptoms()
                .iter()
                .any(|d| d.code.slug() == "codegen-string-too-large")
        );
    }
}

#[test]
fn scalar_literals_reject_incompatible_storage_before_emission() {
    let source = format!(
        "{}identity(value Byte) Byte: return value\nselect() Byte: return 0\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    let mut scope = TransformCtx::new();
    scope.compile_target = target.clone();
    let parsed = parser::parse_from_str(&source);
    assert!(parsed.parse_warnings.is_empty());
    let original = transform(&parsed, FileId(0), &scope);
    assert!(original.all_flaws().is_empty());
    let byte = original.defs[&DefId::named(Intern::from_ref("identity"))].params[0]
        .1
        .clone();
    let returned = match original.defs[&DefId::named(Intern::from_ref("select"))].body {
        typecheck::BindBody::Expr(id) | typecheck::BindBody::Body { ret: Some(id), .. } => id,
        _ => panic!("return expression"),
    };
    let array = |element, length| Ty::Array {
        elem: Box::new(element),
        size: ast::NormalExpr::from(length),
    };
    for (literal, ty) in [
        (ast::Literal::Float(ast::HashFloat(1.5)), byte.clone()),
        (ast::Literal::String("abc".into()), byte.clone()),
        (ast::Literal::String("abc".into()), array(byte.clone(), 2)),
        (ast::Literal::String("abc".into()), array(byte.clone(), 4)),
        (
            ast::Literal::String("abc".into()),
            array(Ty::bounded_int(0.into(), 65535.into()), 3),
        ),
    ] {
        let mut typed = original.clone();
        typed.exprs.facts.const_facts.const_value[returned.as_usize()] = Some(match &literal {
            ast::Literal::Float(value) => ast::ConstValue::Float(*value),
            ast::Literal::String(value) => ast::ConstValue::Text(value.clone()),
            _ => unreachable!(),
        });
        typed.exprs.kind[returned.as_usize()] = typecheck::TypedExprKind::Lit(literal);
        typed.exprs.ty[returned.as_usize()] = ty;
        let mlir = crate::NativeCompiler::create_context();
        let context = CodegenContext::with_string_prefix(
            &mlir,
            typed.resolved_program(),
            typecheck::layout::TargetLayout::from_compile_target(&target).unwrap(),
            CodegenSourceMap::new("scalar_literal.gin", &source, &typed.span_table),
            "",
        );
        let region = Region::new();
        region.append_block(Block::new(&[]));
        let block = region.first_block().unwrap();
        let mut symbols = ScopedSymbolTable::new();
        assert!(
            context
                .lower_typed_expr(returned, &block, &mut symbols)
                .is_none()
        );
        assert!(block.first_operation().is_none());
        assert!(context.globals.string_symbols().is_empty());
        assert!(
            context
                .drain_symptoms()
                .iter()
                .any(|d| d.code.slug() == "codegen-constant-type-mismatch")
        );
        if matches!(
            typed.exprs.kind[returned.as_usize()],
            typecheck::TypedExprKind::Lit(ast::Literal::String(_))
        ) {
            assert!(
                context
                    .lower_typed_place(returned, &block, &mut symbols)
                    .is_none()
            );
            assert!(block.first_operation().is_none());
            assert!(context.globals.string_symbols().is_empty());
            assert!(
                context
                    .drain_symptoms()
                    .iter()
                    .any(|d| d.code.slug() == "codegen-constant-type-mismatch")
            );
        }
    }
}

#[test]
#[allow(unsafe_code)]
fn folded_integer_constants_preserve_selected_width_and_bits() {
    for (bounds, input, width, encoded) in [
        ("0 to 255".to_owned(), "255".to_owned(), 8, "-1".to_owned()),
        (
            "-128 to 127".to_owned(),
            "-42".to_owned(),
            8,
            "-42".to_owned(),
        ),
        (
            format!("0 to {}", (1u128 << 65) - 1),
            u64::MAX.to_string(),
            65,
            u64::MAX.to_string(),
        ),
        (
            format!("0 to {}", u128::MAX),
            (1u128 << 127).to_string(),
            128,
            i128::MIN.to_string(),
        ),
        (
            format!("0 to {}", u128::MAX),
            u128::MAX.to_string(),
            128,
            "-1".to_owned(),
        ),
        (
            format!("{} to {}", i128::MIN, i128::MAX),
            i128::MIN.to_string(),
            128,
            i128::MIN.to_string(),
        ),
    ] {
        let source = format!(
            "Selected is in {bounds}\nSelected has\n    finish(self):\n        return\nidentity(value Selected) Selected: return value\n"
        );
        let parsed = parser::parse_from_str(&source);
        assert!(parsed.parse_warnings.is_empty());
        let target = flask::CompileTarget::Concrete(
            flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
        );
        let mut scope = TransformCtx::new();
        scope.compile_target = target.clone();
        let typed = transform(&parsed, FileId(0), &scope);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let ty = &typed.defs[&DefId::named(Intern::from_ref("identity"))].params[0].1;
        let mlir = crate::NativeCompiler::create_context();
        let context = CodegenContext::with_string_prefix(
            &mlir,
            typed.resolved_program(),
            typecheck::layout::TargetLayout::from_compile_target(&target).unwrap(),
            CodegenSourceMap::new("constant.gin", &source, &typed.span_table),
            "",
        );
        let region = Region::new();
        region.append_block(Block::new(&[]));
        let block = region.first_block().unwrap();
        let value = context
            .lower_const_value(&block, &ast::ConstValue::Int(input.parse().unwrap()), ty)
            .expect("selected integer constant");
        assert_eq!(value.r#type().to_string(), format!("i{width}"));
        let operation = block.first_operation().unwrap();
        assert_eq!(
            operation.attribute("value").unwrap().to_string(),
            format!("{encoded} : i{width}")
        );
        block.append_operation(melior::dialect::func::r#return(
            &[value],
            Location::unknown(&mlir),
        ));
        let module = melior::ir::Module::new(Location::unknown(&mlir));
        module.body().append_operation(melior::dialect::func::func(
            &mlir,
            melior::ir::attribute::StringAttribute::new(&mlir, "selected_constant"),
            melior::ir::attribute::TypeAttribute::new(
                melior::ir::r#type::FunctionType::new(&mlir, &[], &[value.r#type()]).into(),
            ),
            region,
            &[(
                Identifier::new(&mlir, "llvm.emit_c_interface"),
                Attribute::unit(&mlir),
            )],
            Location::unknown(&mlir),
        ));
        assert!(module.as_operation().verify());
        let engine = crate::NativeEngine::new_with_export_abi(module, "selected_constant").unwrap();
        let expected_bits = input
            .parse::<u128>()
            .unwrap_or_else(|_| input.parse::<i128>().unwrap() as u128);
        let mut result = 0u128;
        unsafe { engine.invoke_packed(&mut [(&mut result as *mut u128).cast()]) }.unwrap();
        let mask = if width == 128 {
            u128::MAX
        } else {
            (1u128 << width) - 1
        };
        assert_eq!(result & mask, expected_bits & mask);
        assert!(context.drain_symptoms().is_empty());
    }
}

#[test]
fn constants_reject_incompatible_storage_before_emission() {
    let source = format!(
        "{}identity(value Byte) Byte: return value\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    let mut scope = TransformCtx::new();
    scope.compile_target = target.clone();
    let parsed = parser::parse_from_str(&source);
    assert!(parsed.parse_warnings.is_empty());
    let typed = transform(&parsed, FileId(0), &scope);
    assert!(typed.all_flaws().is_empty());
    let byte = typed.defs[&DefId::named(Intern::from_ref("identity"))].params[0]
        .1
        .clone();
    let array = |length| ast::Ty::Array {
        elem: Box::new(byte.clone()),
        size: ast::NormalExpr::from(length),
    };
    let mlir = crate::NativeCompiler::create_context();
    let context = CodegenContext::with_string_prefix(
        &mlir,
        typed.resolved_program(),
        typecheck::layout::TargetLayout::from_compile_target(&target).unwrap(),
        CodegenSourceMap::new("constant.gin", &source, &typed.span_table),
        "",
    );
    for (value, ty, code) in [
        (
            ast::ConstValue::Int(256.into()),
            byte.clone(),
            "codegen-integer-constant-domain",
        ),
        (
            ast::ConstValue::Int(1.into()),
            ast::Ty::Unit,
            "codegen-constant-type-mismatch",
        ),
        (
            ast::ConstValue::Float(ast::HashFloat(1.5)),
            byte.clone(),
            "codegen-constant-type-mismatch",
        ),
        (
            ast::ConstValue::Text("abc".into()),
            byte.clone(),
            "codegen-constant-type-mismatch",
        ),
        (
            ast::ConstValue::Text("abc".into()),
            array(2i128),
            "codegen-constant-type-mismatch",
        ),
        (
            ast::ConstValue::Int("680564733841876926926749214863536422912".parse().unwrap()),
            byte.clone(),
            "codegen-integer-constant-width",
        ),
    ] {
        let region = Region::new();
        region.append_block(Block::new(&[]));
        let block = region.first_block().unwrap();
        assert!(context.lower_const_value(&block, &value, &ty).is_none());
        assert!(block.first_operation().is_none());
        let diagnostics = context.drain_symptoms();
        assert!(
            diagnostics.iter().any(|d| d.code.slug() == code),
            "{diagnostics:?}"
        );
    }
    let region = Region::new();
    region.append_block(Block::new(&[]));
    let block = region.first_block().unwrap();
    let value = context
        .lower_const_value(&block, &ast::ConstValue::Text("abc".into()), &array(3))
        .unwrap();
    assert_eq!(value.r#type().to_string(), "!llvm.array<3 x i8>");
    assert!(context.drain_symptoms().is_empty());
}
