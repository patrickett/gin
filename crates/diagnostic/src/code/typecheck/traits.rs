use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum TraitCode {
    AmbiguousDefault,
    ArgumentCount,
    MethodMismatch,
    MissingMethod,
    ParameterConstraint,
    UninheritedQualifier,
    Unknown,
    // type-unknown-trait-method
    UnknownMethod,
}

impl Prefix for TraitCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for TraitCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AmbiguousDefault => "ambiguous-trait-default",
            Self::ArgumentCount => "trait-argument-count",
            Self::MethodMismatch => "trait-method-mismatch",
            Self::MissingMethod => "missing-trait-method",
            Self::ParameterConstraint => "parameter-trait-constraint",
            Self::UninheritedQualifier => "uninherited-trait-qualifier",
            Self::Unknown => "unknown-trait",
            Self::UnknownMethod => "unknown-trait-method",
        }
    }
}

impl TraitCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AmbiguousDefault,
            Self::ArgumentCount,
            Self::MethodMismatch,
            Self::MissingMethod,
            Self::ParameterConstraint,
            Self::UninheritedQualifier,
            Self::Unknown,
            Self::UnknownMethod,
        ]
        .into_iter()
    }
}

impl From<TraitCode> for crate::DiagnosticCode {
    fn from(code: TraitCode) -> Self {
        TypeCode::Trait(code).into()
    }
}
