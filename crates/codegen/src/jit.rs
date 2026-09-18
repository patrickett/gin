use crate::emit::NativeCompiler;
use diagnostic::Diagnostic;
use melior::{
    ExecutionEngine,
    ir::{
        Attribute, BlockLike, Module,
        attribute::{StringAttribute, TypeAttribute},
        operation::{OperationLike, OperationMutLike},
        r#type::{FunctionType, IntegerType},
    },
};
use thiserror::Error;

pub struct NativeEngine {
    engine: ExecutionEngine,
    _compiler: NativeCompiler,
    export: String,
}

#[derive(Debug, Error)]
pub enum JitError {
    #[error("cannot JIT an invalid MLIR module")]
    InvalidModule,
    #[error("failed to prepare MLIR module for JIT: {message}", message = .0.message)]
    ModulePreparation(Box<Diagnostic>),
    #[error("cannot JIT target-specific module `{target}` on this host")]
    TargetMismatch { target: String },
    #[error("JIT export `{symbol}` was not found")]
    ExportNotFound { symbol: String },
    #[error("JIT export `{symbol}` must have ABI `() -> i64`, found `{actual}`")]
    InvalidExportAbi { symbol: String, actual: String },
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
        Self::new_inner(module, export.into(), true)
    }

    pub fn new_with_export_abi(
        module: Module<'_>,
        export: impl Into<String>,
    ) -> Result<Self, JitError> {
        Self::new_inner(module, export.into(), false)
    }

    fn new_inner(module: Module<'_>, export: String, validate_i64: bool) -> Result<Self, JitError> {
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

        if validate_i64 {
            mark_i64_export(&mut module, compiler.context(), &export)?;
        } else {
            mark_export(&mut module, compiler.context(), &export)?;
        }

        let mut diagnostics = Vec::new();
        if !compiler.lower_to_llvm(&mut module, &mut diagnostics) {
            return Err(JitError::LoweringFailed {
                diagnostics: diagnostics.into_boxed_slice(),
            });
        }

        let engine = ExecutionEngine::new(&module, 2, &[], false, false);
        drop(module);
        Ok(Self {
            engine,
            _compiler: compiler,
            export,
        })
    }

    #[allow(unsafe_code)]
    pub fn invoke_i64(&self) -> Result<i64, JitError> {
        let mut result = 0i64;
        let mut arguments = [&mut result as *mut i64 as *mut ()];
        self.invoke_packed(&mut arguments)?;
        Ok(result)
    }

    #[allow(unsafe_code)]
    pub fn invoke_ptr(&self) -> Result<*mut (), JitError> {
        let mut result = std::ptr::null_mut();
        let mut arguments = [&mut result as *mut *mut () as *mut ()];
        self.invoke_packed(&mut arguments)?;
        Ok(result)
    }

    #[allow(unsafe_code)]
    pub fn invoke_packed(&self, arguments: &mut [*mut ()]) -> Result<(), JitError> {
        unsafe { self.engine.invoke_packed(&self.export, arguments) }.map_err(|error| {
            JitError::InvocationFailed {
                symbol: self.export.clone(),
                error,
            }
        })
    }

    #[allow(unsafe_code)]
    pub fn lookup_export_address(&self) -> Result<*mut (), JitError> {
        self.lookup_address(&self.export)
    }

    #[allow(unsafe_code)]
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

#[cfg(test)]
mod tests {
    use super::target_matches_host;

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

fn mark_i64_export<'c>(
    module: &mut Module<'c>,
    context: &'c melior::Context,
    export: &str,
) -> Result<(), JitError> {
    let mut current = module.body().first_operation_mut();
    while let Some(mut operation) = current {
        current = operation.next_in_block_mut();
        if operation.name().as_string_ref().as_str() != Ok("func.func") {
            continue;
        }

        let Ok(symbol) = operation
            .attribute("sym_name")
            .and_then(StringAttribute::try_from)
        else {
            continue;
        };
        if symbol.value() != export {
            continue;
        }

        let actual = operation
            .attribute("function_type")
            .map(|attribute| attribute.to_string())
            .unwrap_or_else(|_| "<missing function type>".to_string());
        let valid = operation
            .attribute("function_type")
            .and_then(TypeAttribute::try_from)
            .map(|attribute| attribute.value())
            .and_then(FunctionType::try_from)
            .is_ok_and(|function| {
                function.input_count() == 0
                    && function.result_count() == 1
                    && function
                        .result(0)
                        .and_then(IntegerType::try_from)
                        .is_ok_and(|integer| integer.width() == 64 && integer.is_signless())
            });

        if !valid {
            return Err(JitError::InvalidExportAbi {
                symbol: export.to_string(),
                actual,
            });
        }

        operation.set_attribute("llvm.emit_c_interface", Attribute::unit(context));
        return Ok(());
    }

    Err(JitError::ExportNotFound {
        symbol: export.to_string(),
    })
}

fn mark_export<'c>(
    module: &mut Module<'c>,
    context: &'c melior::Context,
    export: &str,
) -> Result<(), JitError> {
    let mut current = module.body().first_operation_mut();
    while let Some(mut operation) = current {
        current = operation.next_in_block_mut();
        if operation.name().as_string_ref().as_str() != Ok("func.func") {
            continue;
        }
        let Ok(symbol) = operation
            .attribute("sym_name")
            .and_then(StringAttribute::try_from)
        else {
            continue;
        };
        if symbol.value() == export {
            operation.set_attribute("llvm.emit_c_interface", Attribute::unit(context));
            return Ok(());
        }
    }
    Err(JitError::ExportNotFound {
        symbol: export.to_string(),
    })
}
