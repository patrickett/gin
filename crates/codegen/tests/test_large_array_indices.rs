use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, Profile};
use typecheck::transform::{TransformCtx, transform};

fn module<'c>(context: &'c melior::Context, source: &str) -> melior::ir::Module<'c> {
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
    let typed = transform(&parsed, typecheck::FileId(0), &scope);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        context,
        typed.resolved_program(),
        CodegenSourceMap::new("large-index.gin", source, &typed.span_table),
        &target,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    module.unwrap()
}

fn check_indices(write: bool) {
    for index in [2147483647usize, 2147483648, 2147483649, 4294967294] {
        let declaration = if write {
            format!(
                "replace(previous Byte, next Byte) Byte: return next\nwrite(values mut [Byte; {}], replacement Byte):\n    values.{index} :: replace(values.{index}, replacement)\n    return\n",
                index + 1
            )
        } else {
            format!(
                "borrow(values ref [Byte; {}]) ref Byte: return ref values.{index}\n",
                index + 1
            )
        };
        let source = format!("{}{declaration}", test_fixtures::preludes::BYTE_PRELUDE);
        let context = NativeCompiler::create_context();
        let module = module(&context, &source);
        let compiler = NativeCompiler::default();
        let (llvm, diagnostics) = compiler.llvm_ir_from_module(&module, Profile::Debug, true);
        assert!(diagnostics.is_empty(), "{index}: {diagnostics:?}");
        let llvm = llvm.unwrap();
        let width = if i32::try_from(index).is_ok() { 32 } else { 64 };
        assert!(
            llvm.lines().any(|line| line.contains("getelementptr")
                && line.contains(&format!("i{width} {index}"))),
            "{index}: {llvm}"
        );
        if write {
            assert!(llvm.contains("store i8"), "{llvm}");
            assert!(llvm.contains("load i8"), "{llvm}");
            assert!(!llvm.contains("load ["), "{llvm}");
        }
    }
}

#[test]
fn borrowed_array_indices_preserve_unsigned_32_bit_positions() {
    check_indices(false);
}

#[test]
fn mutable_array_indices_preserve_unsigned_32_bit_positions() {
    check_indices(true);
}

#[test]
#[allow(unsafe_code)]
fn constant_projection_mutates_the_borrowed_array_storage() {
    let source = format!(
        "{}replace(previous Byte, next Byte) Byte: return next\nwrite(values mut [Byte; 4], replacement Byte):\n    values.3 :: replace(values.3, replacement)\n    return\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let context = NativeCompiler::create_context();
    let engine =
        codegen::NativeEngine::new_with_export_abi(module(&context, &source), "write").unwrap();
    let mut values = [3u8, 17, 128, 255];
    let mut pointer = values.as_mut_ptr();
    let mut replacement = 42u8;
    unsafe {
        engine.invoke_packed(&mut [
            (&mut pointer as *mut *mut u8).cast(),
            (&mut replacement as *mut u8).cast(),
        ])
    }
    .unwrap();
    assert_eq!(values, [3, 17, 128, 42]);
}
