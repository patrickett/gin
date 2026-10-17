use super::TypeCode;
use crate::DiagnosticCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum IntrinsicCode {
    DivisionByZero,
    InvalidComparisonResult,
    InvalidExtension,
    InvalidTruncation,
    MismatchedWidths,
    NonAddress,
    NonBits,
    SignedDivisionOverflow,
    UnsupportedWidth,
    WrongArity,
}

impl Prefix for IntrinsicCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for IntrinsicCode {
    fn name(&self) -> &'static str {
        match self {
            Self::DivisionByZero => "intrinsic-division-by-zero",
            Self::InvalidComparisonResult => "intrinsic-invalid-comparison-result",
            Self::InvalidExtension => "intrinsic-invalid-extension",
            Self::InvalidTruncation => "intrinsic-invalid-truncation",
            Self::MismatchedWidths => "intrinsic-mismatched-widths",
            Self::NonAddress => "intrinsic-non-address",
            Self::NonBits => "intrinsic-non-bits",
            Self::SignedDivisionOverflow => "intrinsic-signed-division-overflow",
            Self::UnsupportedWidth => "intrinsic-unsupported-width",
            Self::WrongArity => "intrinsic-wrong-arity",
        }
    }
}

impl IntrinsicCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::DivisionByZero,
            Self::InvalidComparisonResult,
            Self::InvalidExtension,
            Self::InvalidTruncation,
            Self::MismatchedWidths,
            Self::NonAddress,
            Self::NonBits,
            Self::SignedDivisionOverflow,
            Self::UnsupportedWidth,
            Self::WrongArity,
        ]
        .into_iter()
    }
}

impl From<IntrinsicCode> for crate::DiagnosticCode {
    fn from(code: IntrinsicCode) -> Self {
        DiagnosticCode::Type(TypeCode::Intrinsic(code))
    }
}
