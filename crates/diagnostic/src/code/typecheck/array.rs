use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum ArrayCode {
    DependentBorrow,
    DependentBounds,
    DependentElement,
    DependentLength,
    DependentMutation,
    DependentPermission,
    FixedElementMismatch,
    FixedInvalidFormer,
    FixedLengthMismatch,
    FixedRequiresExpectedType,
    IndexOutOfBounds,
    LengthInference,
}

impl Prefix for ArrayCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for ArrayCode {
    fn name(&self) -> &'static str {
        match self {
            Self::DependentBorrow => "dependent-array-borrow",
            Self::DependentBounds => "dependent-array-bounds",
            Self::DependentElement => "dependent-array-element",
            Self::DependentLength => "dependent-array-length",
            Self::DependentMutation => "dependent-array-mutation",
            Self::DependentPermission => "dependent-array-permission",
            Self::FixedElementMismatch => "fixed-array-element-mismatch",
            Self::FixedInvalidFormer => "invalid-fixed-array-former",
            Self::FixedLengthMismatch => "fixed-array-length-mismatch",
            Self::FixedRequiresExpectedType => "fixed-array-requires-expected-type",
            Self::IndexOutOfBounds => "array-index-out-of-bounds",
            Self::LengthInference => "array-length-inference",
        }
    }
}

impl ArrayCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::DependentBorrow,
            Self::DependentBounds,
            Self::DependentElement,
            Self::DependentLength,
            Self::DependentMutation,
            Self::DependentPermission,
            Self::FixedElementMismatch,
            Self::FixedInvalidFormer,
            Self::FixedLengthMismatch,
            Self::FixedRequiresExpectedType,
            Self::IndexOutOfBounds,
            Self::LengthInference,
        ]
        .into_iter()
    }
}

impl From<ArrayCode> for crate::DiagnosticCode {
    fn from(code: ArrayCode) -> Self {
        TypeCode::Array(code).into()
    }
}
