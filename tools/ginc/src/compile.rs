use crate::cli::{Args, Emit};
use crate::reporting::CheckOptions;
use diagnostic::Diagnostic;
use std::path::PathBuf;
use strum::Display;
use thiserror::Error;

pub struct GinCompiler;

#[derive(Debug)]
pub enum CompileResult {
    Success { output: Option<PathBuf> },
    Failed(CompileFailure),
}

#[derive(Debug, Error)]
pub enum CompileFailure {
    #[error("invalid compiler invocation: {0}")]
    InvalidOptions(String),
    #[error("package failed: {0}")]
    Package(String),
    #[error("lowering failed: {0}")]
    Lowering(String),
    #[error("failed to read {path}: {error}")]
    UnreadableSource { path: PathBuf, error: String },
    #[error("parse failed")]
    ParseFailed,
    #[error("type checking failed")]
    TypecheckFailed,
    #[error("artifact storage failed: {0}")]
    ArtifactStorage(std::io::Error),
    #[error("{stage} failed")]
    EmissionFailed {
        stage: CompileEmitStage,
        diagnostics: Vec<Diagnostic>,
    },
}

#[derive(Debug, Display)]
pub enum CompileEmitStage {
    #[strum(to_string = "object emission")]
    ObjectEmission,
    #[strum(to_string = "linking")]
    Linking,
}

impl GinCompiler {
    pub fn compile_cli(args: &mut Args) -> CompileResult {
        let builds = matches!(args.emit, Emit::Auto | Emit::Exe | Emit::Obj);
        let output = builds.then(|| crate::build_output::BuildOutput::new(args));
        let result = Self::compile(args);
        if let Some(output) = output {
            output.finish(&result, args);
        }
        result
    }

    pub fn compile(args: &mut Args) -> CompileResult {
        crate::pipeline::run(
            args,
            CheckOptions {
                message_format: args.message_format,
                max_errors: args.max_errors,
            },
            args.emit,
        )
    }

    /// Check only, regardless of the emission mode stored in `args`.
    pub fn check(args: &mut Args, options: CheckOptions) -> CompileResult {
        crate::pipeline::run(args, options, Emit::Check)
    }

    pub(crate) fn fast_llvm_enabled() -> bool {
        env_flag_enabled("GINC_FAST_LLVM")
    }
}

impl CompileResult {
    pub fn has_typecheck_diagnostics(&self) -> bool {
        matches!(self, Self::Failed(CompileFailure::TypecheckFailed))
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }
}

pub(crate) fn env_flag_enabled(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .map(|value| value.to_lowercase())
            .as_deref(),
        Ok("1") | Ok("true") | Ok("on") | Ok("yes")
    )
}
