use codegen::{JitError, NativeCompiler, NativeEngine};
use melior::{
    ir::{BlockLike, operation::OperationLike, operation::OperationMutLike},
    ir::{
        Module,
        attribute::{Attribute, StringAttribute},
    },
};

fn mark_i64_export(
    module: &mut Module<'_>,
    context: &melior::Context,
    export: &str,
) -> Result<(), JitError> {
    let mut operation = module.body().first_operation_mut();
    while let Some(mut current) = operation {
        operation = current.next_in_block_mut();
        if current.name().as_string_ref().as_str() != Ok("func.func") {
            continue;
        }

        let found = current
            .attribute("sym_name")
            .and_then(StringAttribute::try_from)
            .is_ok_and(|symbol| symbol.value() == export);
        if !found {
            continue;
        }

        current.set_attribute("llvm.emit_c_interface", Attribute::unit(context));
        return Ok(());
    }

    Err(JitError::ExportNotFound {
        symbol: export.to_string(),
    })
}

#[test]
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

    assert_eq!(engine.invoke_i64().expect("native result"), 42);
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
fn marks_only_requested_export() {
    let input_context = NativeCompiler::create_context();
    let module = Module::parse(
        &input_context,
        r#"
            module {
                func.func @answer() -> i64 {
                    %result = arith.constant 42 : i64
                    return %result : i64
                }
                func.func @helper() -> i64 {
                    %result = arith.constant 1 : i64
                    return %result : i64
                }
            }
            "#,
    )
    .expect("valid MLIR syntax");
    let mut module = module;

    mark_i64_export(&mut module, &input_context, "answer").expect("valid export");

    let answer = module.body().first_operation().expect("answer");
    let helper = answer.next_in_block().expect("helper");
    assert!(answer.has_attribute("llvm.emit_c_interface"));
    assert!(!helper.has_attribute("llvm.emit_c_interface"));
}
