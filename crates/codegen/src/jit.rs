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
        let export = export.into();
        let compiler = NativeCompiler::default();
        let mut module = compiler
            .reparse_for_backend(&module)
            .map_err(JitError::ModulePreparation)?;
        if !module.as_operation().verify() {
            return Err(JitError::InvalidModule);
        }

        mark_i64_export(&mut module, compiler.context(), &export)?;

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
        unsafe { self.engine.invoke_packed(&self.export, &mut arguments) }.map_err(|error| {
            JitError::InvocationFailed {
                symbol: self.export.clone(),
                error,
            }
        })?;
        Ok(result)
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
