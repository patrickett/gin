use super::JitError;
use melior::{StringRef, ir::Module};
use mlir_sys::MlirExecutionEngine;
use std::{ffi::c_void, ptr::NonNull};

pub(super) struct ExecutionEngine {
    handle: NonNull<c_void>,
}

impl ExecutionEngine {
    #[allow(unsafe_code)]
    pub(super) fn new(module: &Module<'_>) -> Result<Self, JitError> {
        let raw = unsafe {
            mlir_sys::mlirExecutionEngineCreate(
                module.to_raw(),
                2,
                0,
                std::ptr::null(),
                false,
                false,
            )
        };
        let handle = NonNull::new(raw.ptr).ok_or(JitError::EngineCreationFailed)?;
        Ok(Self { handle })
    }

    fn raw(&self) -> MlirExecutionEngine {
        MlirExecutionEngine {
            ptr: self.handle.as_ptr(),
        }
    }

    #[allow(unsafe_code)]
    pub(super) fn lookup(&self, name: &str) -> *mut () {
        unsafe {
            mlir_sys::mlirExecutionEngineLookup(self.raw(), StringRef::new(name).to_raw()).cast()
        }
    }

    /// # Safety
    /// Arguments must match the exported C interface and point to valid storage.
    /// The exported code must preserve Rust memory safety and must not unwind.
    #[allow(unsafe_code)]
    pub(super) unsafe fn invoke_packed(
        &self,
        name: &str,
        arguments: &mut [*mut ()],
    ) -> Result<(), melior::Error> {
        let result = unsafe {
            mlir_sys::mlirExecutionEngineInvokePacked(
                self.raw(),
                StringRef::new(name).to_raw(),
                arguments.as_mut_ptr().cast(),
            )
        };
        if result.value != 0 {
            Ok(())
        } else {
            Err(melior::Error::InvokeFunction)
        }
    }
}

impl Drop for ExecutionEngine {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        unsafe { mlir_sys::mlirExecutionEngineDestroy(self.raw()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use melior::ir::operation::OperationLike;

    #[test]
    fn rejects_engine_creation_failure() {
        let context = crate::NativeCompiler::create_context();
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
        assert!(module.as_operation().verify());
        assert!(matches!(
            ExecutionEngine::new(&module),
            Err(JitError::EngineCreationFailed)
        ));
    }
}
