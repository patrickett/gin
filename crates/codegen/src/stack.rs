use diagnostic::Diagnostic;
use diagnostic::code::CodegenCode;
use melior::ir::{
    BlockLike, Module, RegionLike,
    attribute::IntegerAttribute,
    operation::{OperationLike, OperationRef, OperationResult, WalkOrder, WalkResult},
};

pub(crate) fn verify_stack_storage(module: &Module<'_>) -> Result<(), Diagnostic> {
    let mut failure = None;
    module.as_operation().walk(WalkOrder::PreOrder, |operation| {
        if operation.name().as_string_ref().as_str() == Ok("llvm.alloca")
            && !is_fixed_entry_slot(operation)
        {
            failure = Some(
                // codegen-invalid-stack-storage
                Diagnostic::new(
                    CodegenCode::InvalidStackStorage,
                    format!("fixed-size stack storage must be reserved in the function entry prologue at {}", operation.location()),
                )
                .with_help("an internal compiler error occurred"),
            );
            WalkResult::Interrupt
        } else {
            WalkResult::Advance
        }
    });
    failure.map_or(Ok(()), Err)
}

fn is_fixed_entry_slot(allocation: OperationRef<'_, '_>) -> bool {
    let Some(block) = allocation.block() else {
        return false;
    };
    let Some(function) = block.parent_operation() else {
        return false;
    };
    if !matches!(
        function.name().as_string_ref().as_str(),
        Ok("func.func" | "llvm.func")
    ) {
        return false;
    }
    if function
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
        != Some(block)
    {
        return false;
    }
    let Some(count) = allocation
        .operand(0)
        .ok()
        .and_then(|value| OperationResult::try_from(value).ok())
    else {
        return false;
    };
    let count = count.owner();
    if !is_constant(count)
        || !count
            .attribute("value")
            .ok()
            .and_then(|attribute| IntegerAttribute::try_from(attribute).ok())
            .is_some_and(|value| value.value() > 0)
    {
        return false;
    }
    let mut preceding = block.first_operation();
    while let Some(operation) = preceding {
        if operation == allocation {
            return true;
        }
        if !is_constant(operation) && operation.name().as_string_ref().as_str() != Ok("llvm.alloca")
        {
            return false;
        }
        preceding = operation.next_in_block();
    }
    false
}

fn is_constant(operation: OperationRef<'_, '_>) -> bool {
    matches!(
        operation.name().as_string_ref().as_str(),
        Ok("arith.constant" | "llvm.mlir.constant")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NativeCompiler, Profile};

    #[test]
    fn accepts_fixed_slots_at_function_entry() {
        let compiler = NativeCompiler::default();
        let module = Module::parse(
            compiler.context(),
            r#"module {
            func.func @example() {
                %one = arith.constant 1 : i64
                %slot = llvm.alloca %one x i64 : (i64) -> !llvm.ptr
                return
            }
        }"#,
        )
        .unwrap();
        assert!(module.as_operation().verify());
        assert!(verify_stack_storage(&module).is_ok());
    }

    #[test]
    fn rejects_loop_branch_dynamic_and_late_allocations_before_optimization() {
        for source in [
            r#"module { func.func @example() {
                %zero = arith.constant 0 : index
                %end = arith.constant 10 : index
                %step = arith.constant 1 : index
                %one = arith.constant 1 : i64
                scf.for %i = %zero to %end step %step {
                    %slot = llvm.alloca %one x i64 : (i64) -> !llvm.ptr
                }
                return
            }}"#,
            r#"module { func.func @example(%condition: i1) {
                %one = arith.constant 1 : i64
                scf.if %condition {
                    %slot = llvm.alloca %one x i64 : (i64) -> !llvm.ptr
                }
                return
            }}"#,
            r#"module { func.func @example(%count: i64) {
                %slot = llvm.alloca %count x i64 : (i64) -> !llvm.ptr
                return
            }}"#,
            r#"module { func.func @example() {
                %one = arith.constant 1 : i64
                cf.br ^body
            ^body:
                %slot = llvm.alloca %one x i64 : (i64) -> !llvm.ptr
                return
            }}"#,
            r#"module {
                func.func private @callee()
                func.func @example() {
                    %one = arith.constant 1 : i64
                    func.call @callee() : () -> ()
                    %slot = llvm.alloca %one x i64 : (i64) -> !llvm.ptr
                    return
                }
            }"#,
        ] {
            let compiler = NativeCompiler::default();
            let module = Module::parse(compiler.context(), source).unwrap();
            assert!(module.as_operation().verify(), "{source}");
            assert!(verify_stack_storage(&module).is_err(), "{source}");
            for profile in [Profile::Debug, Profile::Release] {
                let (llvm, diagnostics) = compiler.llvm_ir_from_module(&module, profile, false);
                assert!(llvm.is_none(), "{source}");
                assert!(
                    diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code.slug() == "codegen-invalid-stack-storage")
                );
            }
        }
    }
}
