use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum InterpreterCode {
    CodegenFailure,
    ContextUnavailable,
    EvaluatorStateUnavailable,
    ImportRequiresPackage,
    IncompatibleRedefinition,
    Incomplete,
    InvalidCommand,
    InvalidCommandUsage,
    InvalidTaskId,
    MaterializationContextUnavailable,
    MissingExpression,
    MixedSubmission,
    PrivateSection,
    SaveFailed,
    SharedDeclaration,
    TaskFailed,
    TaskFailure,
    UnknownCommand,
    UnknownSymbol,
}

impl Prefix for InterpreterCode {
    fn prefix(&self) -> &'static str {
        "interpreter"
    }
}

impl Code for InterpreterCode {
    fn name(&self) -> &'static str {
        match self {
            Self::CodegenFailure => "codegen-failure",
            Self::ContextUnavailable => "context-unavailable",
            Self::EvaluatorStateUnavailable => "evaluator-state-unavailable",
            Self::ImportRequiresPackage => "import-requires-package",
            Self::IncompatibleRedefinition => "incompatible-redefinition",
            Self::Incomplete => "incomplete",
            Self::InvalidCommand => "invalid-command",
            Self::InvalidCommandUsage => "invalid-command-usage",
            Self::InvalidTaskId => "invalid-task-id",
            Self::MaterializationContextUnavailable => "materialization-context-unavailable",
            Self::MissingExpression => "missing-expression",
            Self::MixedSubmission => "mixed-submission",
            Self::PrivateSection => "private-section",
            Self::SaveFailed => "save-failed",
            Self::SharedDeclaration => "shared-declaration",
            Self::TaskFailed => "task-failed",
            Self::TaskFailure => "task-failure",
            Self::UnknownCommand => "unknown-command",
            Self::UnknownSymbol => "unknown-symbol",
        }
    }
}

impl InterpreterCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::CodegenFailure,
            Self::ContextUnavailable,
            Self::EvaluatorStateUnavailable,
            Self::ImportRequiresPackage,
            Self::IncompatibleRedefinition,
            Self::Incomplete,
            Self::InvalidCommand,
            Self::InvalidCommandUsage,
            Self::InvalidTaskId,
            Self::MaterializationContextUnavailable,
            Self::MissingExpression,
            Self::MixedSubmission,
            Self::PrivateSection,
            Self::SaveFailed,
            Self::SharedDeclaration,
            Self::TaskFailed,
            Self::TaskFailure,
            Self::UnknownCommand,
            Self::UnknownSymbol,
        ]
        .into_iter()
    }
}
