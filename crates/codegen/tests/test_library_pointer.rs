mod support;

#[test]
fn indexed_addresses_preserve_index_signedness_and_element_stride() {
    check_pointer_offsets(false);
}

#[test]
fn offset_intrinsic_preserves_index_signedness_and_element_stride() {
    check_pointer_offsets(true);
}

#[test]
fn offset_intrinsic_uses_target_index_width() {
    let source = format!(
        "Element is in 0 to {}\nElement has\n    finish(self):\n        return\nIndex is in 0 to {}\nIndex has\n    finish(self):\n        return\nmove_addr(address @Element, index Index) @Element: return @address.[index]\n",
        (1u128 << 65) - 1,
        u128::MAX,
    );
    for (triple, bits) in [("wasm32-unknown-unknown", 32), ("aarch64-apple-darwin", 64)] {
        let target = flask::CompileTarget::Concrete(flask::TargetTriple::parse(triple).unwrap());
        let typed = typed_pointer_program(&source, &target, true);
        let context = codegen::NativeCompiler::create_context();
        let (module, diagnostics) =
            codegen::CodegenContext::build_module_from_resolved_program_for_target(
                &context,
                typed.resolved_program(),
                codegen::CodegenSourceMap::new(
                    "pointer_index_width.gin",
                    &source,
                    &typed.span_table,
                ),
                &target,
            );
        assert!(diagnostics.is_empty(), "{triple}: {diagnostics:?}");
        let text = module.unwrap().as_operation().to_string();
        let geps: Vec<_> = text
            .lines()
            .filter(|line| line.contains("llvm.getelementptr"))
            .collect();
        assert_eq!(geps.len(), 1, "{text}");
        assert!(geps[0].contains(&format!("(!llvm.ptr, i{bits})")), "{text}");
        assert!(geps[0].contains("i65"), "{text}");
        assert!(!text.contains("arith.muli"), "{text}");
    }
}

#[allow(unsafe_code)]
fn check_pointer_offsets(intrinsic: bool) {
    for (width, stride) in [(8, 1), (16, 2), (64, 8), (65, 16), (128, 16)] {
        for index_width in [8, 128] {
            for signed in [false, true] {
                let bounds = if signed {
                    if index_width == 8 {
                        "-128 to 127".to_owned()
                    } else {
                        format!("{} to {}", i128::MIN, i128::MAX)
                    }
                } else if index_width == 8 {
                    "0 to 255".to_owned()
                } else {
                    format!("0 to {}", u128::MAX)
                };
                let maximum = if width == 128 {
                    u128::MAX
                } else {
                    (1u128 << width) - 1
                };
                let source = format!(
                    "Element is in 0 to {maximum}\nElement has\n    finish(self):\n        return\nIndex is in {bounds}\nIndex has\n    finish(self):\n        return\nmove_addr(address @Element, index Index) @Element: return @address.[index]\n"
                );
                let target = test_fixtures::host_compile_target();
                let typed = typed_pointer_program(&source, &target, intrinsic);
                let context = codegen::NativeCompiler::create_context();
                let (module, diagnostics) =
                    codegen::CodegenContext::build_module_from_resolved_program_for_target(
                        &context,
                        typed.resolved_program(),
                        codegen::CodegenSourceMap::new(
                            "pointer_offset.gin",
                            &source,
                            &typed.span_table,
                        ),
                        &target,
                    );
                assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
                let engine =
                    codegen::NativeEngine::new_with_export_abi(module.unwrap(), "move_addr")
                        .unwrap();
                let mut storage = vec![0u128; 1024];
                let mut base = unsafe { storage.as_mut_ptr().cast::<u8>().add(8192) };
                let indices: &[i16] = if signed {
                    &[-128, -1, 0, 127]
                } else {
                    &[0, 1, 127, 128, 255]
                };
                for &index in indices {
                    let mut encoded = index as u8;
                    let mut wide = i128::from(index) as u128;
                    let argument = if index_width == 8 {
                        (&mut encoded as *mut u8).cast()
                    } else {
                        (&mut wide as *mut u128).cast()
                    };
                    let mut result = std::ptr::null_mut::<u8>();
                    unsafe {
                        engine.invoke_packed(&mut [
                            (&mut base as *mut *mut u8).cast(),
                            argument,
                            (&mut result as *mut *mut u8).cast(),
                        ])
                    }
                    .unwrap();
                    let expected = unsafe { base.offset(isize::from(index) * stride) };
                    assert_eq!(
                        result, expected,
                        "width={width}, index_width={index_width}, signed={signed}, index={index}"
                    );
                }
            }
        }
    }
}

fn typed_pointer_program(
    source: &str,
    target: &flask::CompileTarget,
    intrinsic: bool,
) -> typecheck::TypedFileAst {
    let parsed = parser::parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{source}: {:?}",
        parsed.parse_warnings
    );
    let mut scope = typecheck::transform::TransformCtx::new();
    scope.compile_target = target.clone();
    let mut typed = typecheck::transform::transform(&parsed, typecheck::FileId(0), &scope);
    assert!(
        typed.all_flaws().is_empty(),
        "{source}: {:?}",
        typed.all_flaws()
    );
    if intrinsic {
        let bind = &typed.defs[&typecheck::DefId::named(internment::Intern::from_ref("move_addr"))];
        let returned = match bind.body {
            typecheck::BindBody::Expr(id) | typecheck::BindBody::Body { ret: Some(id), .. } => id,
            _ => panic!("return expression"),
        };
        let typecheck::TypedExprKind::TakePtr(element) = typed.exprs.kind[returned.as_usize()]
        else {
            panic!("indexed address");
        };
        let typecheck::TypedExprKind::BufGet { buf, index } = typed.exprs.kind[element.as_usize()]
        else {
            panic!("indexed element");
        };
        let op = typecheck::typed::IntrinsicOp::AddressOffset;
        op.validate_signature(
            &[
                typed.exprs.ty[buf.as_usize()].clone(),
                typed.exprs.ty[index.as_usize()].clone(),
            ],
            &typed.exprs.ty[returned.as_usize()],
            Some(&typed.type_registry),
        )
        .unwrap();
        typed.exprs.kind[returned.as_usize()] = typecheck::TypedExprKind::IntrinsicCall {
            op,
            args: vec![buf, index],
        };
    }
    typed
}

#[test]
fn library_pointer_uses_address_load_store_and_gep_lowering() {
    let source = format!(
        "{}PointerToWord is @Word\nPointerToWord has\n    copy(self ref) Self: return deref self\n    finish(self):\n        return\nread_addr(address PointerToWord) Word: return address.[0]\nreplace(previous Word, next Word) Word: return next\nwrite_addr(address PointerToWord, value Word):\n    pointer PointerToWord: address\n    pointer.[0]:: replace(pointer.[0], value)\n    return\nmove_addr(address PointerToWord, index Word) PointerToWord: return (@address.[index]) as PointerToWord\nentry(address PointerToWord, value Word) Word:\n    moved PointerToWord: move_addr(address, 1)\n    write_addr(PointerToWord.copy(moved), value)\n    return read_addr(moved)\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    typed_pointer_program(&source, &test_fixtures::host_compile_target(), false);
    let (mlir, diagnostics) = support::codegen_to_mlir_text(&source, "library_pointer.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("llvm.load"), "{mlir}");
    assert!(mlir.contains("llvm.store"), "{mlir}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("!llvm.ptr"), "{mlir}");
    assert!(
        !mlir.contains("llvm.extractvalue"),
        "raw pointer library lowering should avoid anonymous-row/aggregate fallback ops: {mlir}"
    );
    assert!(
        !mlir.contains("llvm.insertvalue"),
        "raw pointer library lowering should avoid anonymous-row/aggregate fallback ops: {mlir}"
    );
}

#[test]
fn user_defined_generic_slice_uses_pointer_indexing_lowering() {
    let source = format!(
        "{}Count is in 0 to 18446744073709551615\nCount has\n    finish(self):\n        return\nSlice(x) has\n    pointer @x\n    length Count\n    finish(self):\n        return\nread(value Slice(Byte), index Count) Byte:\n    return value.pointer.[index]\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    typed_pointer_program(&source, &test_fixtures::host_compile_target(), false);
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text(&source, "user_slice_pointer.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("llvm.load"), "{mlir}");
    assert!(mlir.contains("!llvm.ptr"), "{mlir}");
    assert!(
        mlir.contains("llvm.extractvalue"),
        "user-defined slice should extract its pointer field from the nominal record: {mlir}"
    );
    assert!(
        !mlir.contains("llvm.insertvalue"),
        "slice indexing should not construct an aggregate: {mlir}"
    );
}
