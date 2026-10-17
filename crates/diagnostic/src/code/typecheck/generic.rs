use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum GenericCode {
    DuplicateArgument,
    ExcessArguments,
    RecordArguments,
    Specialization,
    SpecializationLimit,
    UnknownArgument,
    UnknownPhantomParameter,
    UnsaturatedApplication,
    UnusedParameter,
}

impl Prefix for GenericCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for GenericCode {
    fn name(&self) -> &'static str {
        match self {
            Self::DuplicateArgument => "duplicate-generic-argument",
            Self::ExcessArguments => "excess-generic-arguments",
            Self::RecordArguments => "record-type-arguments",
            Self::Specialization => "generic-specialization",
            Self::SpecializationLimit => "generic-specialization-limit",
            Self::UnknownArgument => "unknown-generic-argument",
            Self::UnknownPhantomParameter => "unknown-phantom-parameter",
            Self::UnsaturatedApplication => "unsaturated-application",
            Self::UnusedParameter => "unused-generic-parameter",
        }
    }
}

impl GenericCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::DuplicateArgument,
            Self::ExcessArguments,
            Self::RecordArguments,
            Self::Specialization,
            Self::SpecializationLimit,
            Self::UnknownArgument,
            Self::UnknownPhantomParameter,
            Self::UnsaturatedApplication,
            Self::UnusedParameter,
        ]
        .into_iter()
    }
}

impl From<GenericCode> for crate::DiagnosticCode {
    fn from(code: GenericCode) -> Self {
        TypeCode::Generic(code).into()
    }
}
