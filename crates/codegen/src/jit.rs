use crate::emit::NativeCompiler;
use diagnostic::Diagnostic;
use melior::ir::{
    Attribute, BlockLike, Module, RegionLike, Type, TypeLike,
    attribute::{StringAttribute, TypeAttribute},
    operation::{OperationLike, OperationMutLike},
    r#type::{FunctionType, IntegerType},
};
use thiserror::Error;

mod engine;
use engine::ExecutionEngine;

pub struct NativeEngine {
    engine: ExecutionEngine,
    _compiler: NativeCompiler,
    export: String,
    signature: ExportSignature,
    packed_arguments: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TypedExportAbi {
    I64,
    Pointer,
}

impl TypedExportAbi {
    fn description(self) -> &'static str {
        match self {
            Self::I64 => "() -> i64",
            Self::Pointer => "() -> !llvm.ptr",
        }
    }
}

struct ExportSignature {
    typed: Option<TypedExportAbi>,
    actual: String,
}

impl ExportSignature {
    fn require(&self, symbol: &str, expected: TypedExportAbi) -> Result<(), JitError> {
        if self.typed == Some(expected) {
            Ok(())
        } else {
            Err(JitError::InvalidExportAbi {
                symbol: symbol.to_owned(),
                expected: expected.description(),
                actual: self.actual.clone(),
            })
        }
    }
}

#[derive(Debug, Error)]
pub enum JitError {
    #[error("cannot JIT an invalid MLIR module")]
    InvalidModule,
    #[error("failed to create the native execution engine")]
    EngineCreationFailed,
    #[error("failed to prepare MLIR module for JIT: {message}", message = .0.message)]
    ModulePreparation(Box<Diagnostic>),
    #[error("cannot JIT target-specific module `{target}` on this host")]
    TargetMismatch { target: String },
    #[error("JIT export `{symbol}` was not found")]
    ExportNotFound { symbol: String },
    #[error("JIT export `{symbol}` must have ABI `{expected}`, found `{actual}`")]
    InvalidExportAbi {
        symbol: String,
        expected: &'static str,
        actual: String,
    },
    #[error("JIT export `{symbol}` has no function body")]
    UndefinedExport { symbol: String },
    #[error("JIT export `{symbol}` requires {expected} packed pointers, got {actual}")]
    InvalidPackedArguments {
        symbol: String,
        expected: usize,
        actual: usize,
    },
    #[error("failed to lower MLIR module for JIT execution")]
    LoweringFailed { diagnostics: Box<[Diagnostic]> },
    #[error("failed to invoke JIT export `{symbol}`: {error}")]
    InvocationFailed {
        symbol: String,
        error: melior::Error,
    },
}

impl NativeEngine {
    pub fn new(module: Module<'_>, export: impl Into<String>) -> Result<Self, JitError> {
        Self::new_inner(module, export.into(), Some(TypedExportAbi::I64))
    }

    pub fn new_with_export_abi(
        module: Module<'_>,
        export: impl Into<String>,
    ) -> Result<Self, JitError> {
        Self::new_inner(module, export.into(), None)
    }

    fn new_inner(
        module: Module<'_>,
        export: String,
        expected: Option<TypedExportAbi>,
    ) -> Result<Self, JitError> {
        let compiler = NativeCompiler::default();
        let mut module = compiler
            .reparse_for_backend(&module)
            .map_err(JitError::ModulePreparation)?;
        if !module.as_operation().verify() {
            return Err(JitError::InvalidModule);
        }
        if let Some(target) = module
            .as_operation()
            .attribute("llvm.target_triple")
            .ok()
            .and_then(|attribute| StringAttribute::try_from(attribute).ok())
            .map(|attribute| attribute.value().to_string())
            && !target_matches_host(&target)
        {
            return Err(JitError::TargetMismatch { target });
        }

        let signature = mark_export(&mut module, compiler.context(), &export)?;
        if let Some(expected) = expected {
            signature.require(&export, expected)?;
        }

        let mut diagnostics = Vec::new();
        if !compiler.lower_to_llvm(&mut module, &mut diagnostics) {
            return Err(JitError::LoweringFailed {
                diagnostics: diagnostics.into_boxed_slice(),
            });
        }

        let packed_arguments = packed_argument_count(&module, compiler.context(), &export);
        let engine = ExecutionEngine::new(&module)?;
        drop(module);
        Ok(Self {
            engine,
            _compiler: compiler,
            export,
            signature,
            packed_arguments,
        })
    }

    /// # Safety
    /// The exported code must obey Rust's memory safety requirements and must not
    /// unwind into Rust. ABI validation does not establish the safety of its body.
    #[allow(unsafe_code)]
    pub unsafe fn invoke_i64(&self) -> Result<i64, JitError> {
        self.signature.require(&self.export, TypedExportAbi::I64)?;
        let mut result = 0i64;
        let mut arguments = [&mut result as *mut i64 as *mut ()];
        unsafe { self.invoke_packed(&mut arguments) }?;
        Ok(result)
    }

    /// # Safety
    /// The exported code must obey Rust's memory safety requirements and must not
    /// unwind into Rust. The returned pointer's validity is determined by that code.
    #[allow(unsafe_code)]
    pub unsafe fn invoke_ptr(&self) -> Result<*mut (), JitError> {
        self.signature
            .require(&self.export, TypedExportAbi::Pointer)?;
        let mut result = std::ptr::null_mut();
        let mut arguments = [&mut result as *mut *mut () as *mut ()];
        unsafe { self.invoke_packed(&mut arguments) }?;
        Ok(result)
    }

    /// # Safety
    /// Each pointer must address live, aligned storage matching the lowered C
    /// interface's parameters, followed by result storage if it returns a value.
    /// An indirect result parameter contains a pointer to writable result storage.
    /// Inputs must be initialized and result storage must be writable. Referenced
    /// memory must remain valid for the call, and the exported code must obey Rust's
    /// memory safety requirements and must not unwind into Rust.
    #[allow(unsafe_code)]
    pub unsafe fn invoke_packed(&self, arguments: &mut [*mut ()]) -> Result<(), JitError> {
        if arguments.len() != self.packed_arguments {
            return Err(JitError::InvalidPackedArguments {
                symbol: self.export.clone(),
                expected: self.packed_arguments,
                actual: arguments.len(),
            });
        }
        unsafe { self.engine.invoke_packed(&self.export, arguments) }.map_err(|error| {
            JitError::InvocationFailed {
                symbol: self.export.clone(),
                error,
            }
        })
    }

    pub fn lookup_export_address(&self) -> Result<*mut (), JitError> {
        self.lookup_address(&self.export)
    }

    pub fn lookup_address(&self, symbol: &str) -> Result<*mut (), JitError> {
        let address = self.engine.lookup(symbol);
        if address.is_null() {
            Err(JitError::ExportNotFound {
                symbol: symbol.to_string(),
            })
        } else {
            Ok(address)
        }
    }
}

fn target_matches_host(target: &str) -> bool {
    let arch_matches = match std::env::consts::ARCH {
        "x86_64" => target.starts_with("x86_64-"),
        "aarch64" => target.starts_with("aarch64-") || target.starts_with("arm64-"),
        arch => target.starts_with(&format!("{arch}-")),
    };
    let os_matches = match std::env::consts::OS {
        "linux" => target.contains("-linux"),
        "macos" => target.contains("-darwin") || target.contains("-macos"),
        "windows" => target.contains("-windows"),
        os => target.contains(os),
    };
    arch_matches && os_matches
}

#[allow(unsafe_code)]
fn packed_argument_count(module: &Module<'_>, context: &melior::Context, export: &str) -> usize {
    let wrapper_name = format!("_mlir_ciface_{export}");
    let mut current = module.body().first_operation();
    while let Some(operation) = current {
        current = operation.next_in_block();
        if operation.name().as_string_ref().as_str() != Ok("llvm.func") {
            continue;
        }
        let symbol = operation
            .attribute("sym_name")
            .and_then(StringAttribute::try_from)
            .expect("lowered LLVM function has a string symbol name");
        if symbol.value() != wrapper_name {
            continue;
        }
        let function = operation
            .attribute("function_type")
            .and_then(TypeAttribute::try_from)
            .expect("lowered LLVM function has a function type")
            .value();
        let inputs = unsafe { mlir_sys::mlirLLVMFunctionTypeGetNumInputs(function.to_raw()) };
        let result = unsafe {
            Type::from_raw(mlir_sys::mlirLLVMFunctionTypeGetReturnType(
                function.to_raw(),
            ))
        };
        return usize::try_from(inputs).expect("LLVM function has a nonnegative parameter count")
            + usize::from(result != melior::dialect::llvm::r#type::void(context));
    }
    panic!("successful function lowering must produce C interface `{wrapper_name}`");
}

fn mark_export<'c>(
    module: &mut Module<'c>,
    context: &'c melior::Context,
    export: &str,
) -> Result<ExportSignature, JitError> {
    let mut current = module.body().first_operation_mut();
    while let Some(mut operation) = current {
        current = operation.next_in_block_mut();
        if operation.name().as_string_ref().as_str() != Ok("func.func") {
            continue;
        }
        let symbol = operation
            .attribute("sym_name")
            .and_then(StringAttribute::try_from)
            .expect("verified function has a string symbol name");
        if symbol.value() != export {
            continue;
        }
        if operation
            .region(0)
            .expect("verified function has a body region")
            .first_block()
            .is_none()
        {
            return Err(JitError::UndefinedExport {
                symbol: export.to_owned(),
            });
        }
        let function = operation
            .attribute("function_type")
            .and_then(TypeAttribute::try_from)
            .map(|attribute| attribute.value())
            .and_then(FunctionType::try_from)
            .expect("verified function has a function type");
        let typed = if function.input_count() == 0 && function.result_count() == 1 {
            let result = function.result(0).expect("function declares one result");
            if IntegerType::try_from(result)
                .is_ok_and(|integer| integer.width() == 64 && integer.is_signless())
            {
                Some(TypedExportAbi::I64)
            } else if result == melior::dialect::llvm::r#type::pointer(context, 0) {
                Some(TypedExportAbi::Pointer)
            } else {
                None
            }
        } else {
            None
        };
        let signature = ExportSignature {
            typed,
            actual: function.to_string(),
        };
        operation.set_attribute("llvm.emit_c_interface", Attribute::unit(context));
        return Ok(signature);
    }
    Err(JitError::ExportNotFound {
        symbol: export.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::target_matches_host;

    #[test]
    fn marks_only_requested_defined_export() {
        use melior::ir::{BlockLike, operation::OperationLike};
        let context = crate::NativeCompiler::create_context();
        let mut module = melior::ir::Module::parse(
            &context,
            r#"module {
            func.func @answer() -> i64 {
                %result = arith.constant 42 : i64
                return %result : i64
            }
            func.func @helper() -> i64 {
                %result = arith.constant 1 : i64
                return %result : i64
            }
        }"#,
        )
        .unwrap();
        assert!(module.as_operation().verify());
        super::mark_export(&mut module, &context, "answer").unwrap();
        let answer = module.body().first_operation().unwrap();
        let helper = answer.next_in_block().unwrap();
        assert!(answer.has_attribute("llvm.emit_c_interface"));
        assert!(!helper.has_attribute("llvm.emit_c_interface"));
    }

    #[test]
    fn target_matching_is_host_specific() {
        let host = format!(
            "{}-unknown-{}",
            std::env::consts::ARCH,
            std::env::consts::OS
        );
        assert!(target_matches_host(&host));
        assert!(
            !target_matches_host("aarch64-unknown-linux-gnu")
                || std::env::consts::ARCH == "aarch64"
        );
    }
}
