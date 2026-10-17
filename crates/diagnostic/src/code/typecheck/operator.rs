use super::TypeCode;
use crate::DiagnosticCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum OperatorCode {
    Ambiguous,
    EqualityMismatchedOperands,
    EqualityUnsupported,
    FixedNumericComparison,
    InvalidInfixDeclaration,
    InvalidResult,
    NoVisibleContract,
    NominalUnavailable,
    NotProvided,
    UnknownInfix,
}

impl Prefix for OperatorCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for OperatorCode {
    fn name(&self) -> &'static str {
        match self {
            Self::Ambiguous => "ambiguous-operator",
            Self::EqualityMismatchedOperands => "equality-mismatched-operands",
            Self::EqualityUnsupported => "equality-unsupported",
            Self::FixedNumericComparison => "fixed-numeric-comparison",
            Self::InvalidInfixDeclaration => "invalid-infix-declaration",
            Self::InvalidResult => "invalid-operator-result",
            Self::NoVisibleContract => "no-visible-operator-contract",
            Self::NominalUnavailable => "nominal-operator-unavailable",
            Self::NotProvided => "operator-not-provided",
            Self::UnknownInfix => "unknown-infix-operator",
        }
    }
}

impl OperatorCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::Ambiguous,
            Self::EqualityMismatchedOperands,
            Self::EqualityUnsupported,
            Self::FixedNumericComparison,
            Self::InvalidInfixDeclaration,
            Self::InvalidResult,
            Self::NoVisibleContract,
            Self::NominalUnavailable,
            Self::NotProvided,
            Self::UnknownInfix,
        ]
        .into_iter()
    }
}

impl From<OperatorCode> for crate::DiagnosticCode {
    fn from(code: OperatorCode) -> Self {
        DiagnosticCode::Type(TypeCode::Operator(code))
    }
}
