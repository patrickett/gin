use codegen::{JitError, NativeCompiler, NativeEngine};
use melior::ir::Module;

#[test]
#[allow(unsafe_code)]
fn invokes_zero_argument_i64_function() {
    let context = NativeCompiler::create_context();
    let module = Module::parse(
        &context,
        r#"
            module {
                func.func @answer() -> i64 {
                    %result = arith.constant 42 : i64
                    return %result : i64
                }
            }
            "#,
    )
    .expect("valid MLIR syntax");

    let engine = NativeEngine::new(module, "answer").expect("JIT engine");

    assert_eq!(unsafe { engine.invoke_i64() }.expect("native result"), 42);
}

#[test]
fn loads_non_i64_export_for_generic_abi_callbacks() {
    let context = NativeCompiler::create_context();
    let module = Module::parse(
        &context,
        r#"
            module {
                func.func @render() -> i32 {
                    %result = arith.constant 1 : i32
                    return %result : i32
                }
            }
            "#,
    )
    .expect("valid MLIR syntax");

    let engine = NativeEngine::new_with_export_abi(module, "render").expect("JIT engine");
    assert!(
        !engine
            .lookup_export_address()
            .expect("render address")
            .is_null()
    );
}

#[test]
fn reports_missing_export() {
    let context = NativeCompiler::create_context();
    let module = Module::parse(
        &context,
        r#"
            module {
                func.func @answer() -> i64 {
                    %result = arith.constant 42 : i64
                    return %result : i64
                }
            }
            "#,
    )
    .expect("valid MLIR syntax");

    let error = match NativeEngine::new(module, "missing") {
        Ok(_) => panic!("missing export should fail"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        JitError::ExportNotFound { symbol } if symbol == "missing"
    ));
}

#[test]
fn rejects_non_i64_export() {
    let context = NativeCompiler::create_context();
    let module = Module::parse(
        &context,
        r#"
            module {
                func.func @answer() -> i32 {
                    %result = arith.constant 42 : i32
                    return %result : i32
                }
            }
            "#,
    )
    .expect("valid MLIR syntax");

    let error = match NativeEngine::new(module, "answer") {
        Ok(_) => panic!("ABI mismatch should fail"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        JitError::InvalidExportAbi { symbol, .. } if symbol == "answer"
    ));
}

#[test]
#[allow(unsafe_code)]
fn generic_exports_reject_incompatible_typed_invocations() {
    let context = NativeCompiler::create_context();
    let module = Module::parse(
        &context,
        r#"module {
        func.func @answer() -> i32 {
            %result = arith.constant 42 : i32
            return %result : i32
        }
    }"#,
    )
    .unwrap();
    let engine = NativeEngine::new_with_export_abi(module, "answer").unwrap();
    assert!(matches!(
        unsafe { engine.invoke_i64() },
        Err(JitError::InvalidExportAbi { .. })
    ));
    assert!(matches!(
        unsafe { engine.invoke_ptr() },
        Err(JitError::InvalidExportAbi { .. })
    ));
}

#[test]
#[allow(unsafe_code)]
fn generic_i64_export_keeps_its_checked_typed_abi() {
    let context = NativeCompiler::create_context();
    let module = Module::parse(
        &context,
        r#"module {
        func.func @answer() -> i64 {
            %result = arith.constant 42 : i64
            return %result : i64
        }
    }"#,
    )
    .unwrap();
    let engine = NativeEngine::new_with_export_abi(module, "answer").unwrap();
    assert_eq!(unsafe { engine.invoke_i64() }.unwrap(), 42);
    assert!(matches!(
        unsafe { engine.invoke_ptr() },
        Err(JitError::InvalidExportAbi {
            expected: "() -> !llvm.ptr",
            ..
        })
    ));
}

#[test]
#[allow(unsafe_code)]
fn pointer_export_is_invoked_with_pointer_storage() {
    let context = NativeCompiler::create_context();
    let module = Module::parse(
        &context,
        r#"module {
        func.func @null() -> !llvm.ptr {
            %result = llvm.mlir.zero : !llvm.ptr
            return %result : !llvm.ptr
        }
    }"#,
    )
    .unwrap();
    let engine = NativeEngine::new_with_export_abi(module, "null").unwrap();
    assert!(unsafe { engine.invoke_ptr() }.unwrap().is_null());
    assert!(matches!(
        unsafe { engine.invoke_i64() },
        Err(JitError::InvalidExportAbi {
            expected: "() -> i64",
            ..
        })
    ));
}

#[test]
#[allow(unsafe_code)]
fn packed_invocation_checks_count_and_preserves_inputs() {
    let context = NativeCompiler::create_context();
    let module = Module::parse(
        &context,
        r#"module {
        func.func @add(%a: i64, %b: i64) -> i64 {
            %result = arith.addi %a, %b : i64
            return %result : i64
        }
    }"#,
    )
    .unwrap();
    let engine = NativeEngine::new_with_export_abi(module, "add").unwrap();
    assert!(matches!(
        unsafe { engine.invoke_i64() },
        Err(JitError::InvalidExportAbi { .. })
    ));
    let mut a = 19i64;
    let mut b = 23i64;
    let mut result = -1i64;
    let mut extra = 0i64;
    let mut arguments = [
        (&mut a as *mut i64).cast(),
        (&mut b as *mut i64).cast(),
        (&mut result as *mut i64).cast(),
        (&mut extra as *mut i64).cast(),
    ];
    for count in [0, 1, 2, 4] {
        assert!(
            matches!(unsafe { engine.invoke_packed(&mut arguments[..count]) },
                Err(JitError::InvalidPackedArguments { expected: 3, actual, .. }) if actual == count
            )
        );
        assert_eq!((a, b, result, extra), (19, 23, -1, 0));
    }
    unsafe { engine.invoke_packed(&mut arguments[..3]) }.unwrap();
    assert_eq!((a, b, result, extra), (19, 23, 42, 0));
}

#[test]
#[allow(unsafe_code)]
fn packed_unit_export_requires_no_result_pointer() {
    let context = NativeCompiler::create_context();
    let module = Module::parse(&context, "module { func.func @noop() { return } }").unwrap();
    let engine = NativeEngine::new_with_export_abi(module, "noop").unwrap();
    assert!(matches!(
        unsafe { engine.invoke_i64() },
        Err(JitError::InvalidExportAbi { .. })
    ));
    unsafe { engine.invoke_packed(&mut []) }.unwrap();
    let mut unused = 0i64;
    assert!(matches!(
        unsafe { engine.invoke_packed(&mut [(&mut unused as *mut i64).cast()]) },
        Err(JitError::InvalidPackedArguments {
            expected: 0,
            actual: 1,
            ..
        })
    ));
}

#[test]
#[allow(unsafe_code)]
fn packed_multiple_results_use_the_c_interfaces_indirect_result_pointer() {
    #[repr(C)]
    struct Pair {
        first: i64,
        second: i32,
    }
    let context = NativeCompiler::create_context();
    let module = Module::parse(
        &context,
        r#"module {
        func.func @pair(%first: i64) -> (i64, i32) {
            %second = arith.constant 17 : i32
            return %first, %second : i64, i32
        }
    }"#,
    )
    .unwrap();
    let engine = NativeEngine::new_with_export_abi(module, "pair").unwrap();
    assert!(matches!(
        unsafe { engine.invoke_i64() },
        Err(JitError::InvalidExportAbi { .. })
    ));
    let mut result = Pair {
        first: 0,
        second: 0,
    };
    let mut result_pointer = &mut result as *mut Pair;
    let mut first = 42i64;
    unsafe {
        engine.invoke_packed(&mut [
            (&mut result_pointer as *mut *mut Pair).cast(),
            (&mut first as *mut i64).cast(),
        ])
    }
    .unwrap();
    assert_eq!((result.first, result.second), (42, 17));
}

#[test]
fn declarations_cannot_be_selected_as_exports() {
    for generic in [false, true] {
        let context = NativeCompiler::create_context();
        let module =
            Module::parse(&context, "module { func.func private @external() -> i64 }").unwrap();
        let result = if generic {
            NativeEngine::new_with_export_abi(module, "external")
        } else {
            NativeEngine::new(module, "external")
        };
        assert!(
            matches!(result, Err(JitError::UndefinedExport { symbol }) if symbol == "external")
        );
    }
}
