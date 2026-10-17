use super::TypeCode;
use crate::DiagnosticCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum ReferenceCode {
    AmbiguousReturnTargetGroup,
    InvalidFormer,
    KindIncompatible,
    ReturnLocalTarget,
    ReturnUnknownTarget,
    ReturnWrongTargetGroup,
    StoreLocalTarget,
    ToTemporary,
    UseAfterInvalidation,
    UseAfterPossibleInvalidation,
}

impl Prefix for ReferenceCode {
    fn prefix(&self) -> &'static str {
        match self {
            Self::KindIncompatible => "",
            _ => "type",
        }
    }
}

impl Code for ReferenceCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AmbiguousReturnTargetGroup => "ambiguous-reference-return-target-group",
            Self::InvalidFormer => "invalid-reference-former",
            Self::KindIncompatible => "incompatible-reference-kind",
            Self::ReturnLocalTarget => "reference-return-local-target",
            Self::ReturnUnknownTarget => "reference-return-unknown-target",
            Self::ReturnWrongTargetGroup => "reference-return-wrong-target-group",
            Self::StoreLocalTarget => "reference-store-local-target",
            Self::ToTemporary => "reference-to-temporary",
            Self::UseAfterInvalidation => "use-of-invalidated-reference",
            Self::UseAfterPossibleInvalidation => "use-of-maybe-invalidated-reference",
        }
    }
}

impl ReferenceCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AmbiguousReturnTargetGroup,
            Self::InvalidFormer,
            Self::KindIncompatible,
            Self::ReturnLocalTarget,
            Self::ReturnUnknownTarget,
            Self::ReturnWrongTargetGroup,
            Self::StoreLocalTarget,
            Self::ToTemporary,
            Self::UseAfterInvalidation,
            Self::UseAfterPossibleInvalidation,
        ]
        .into_iter()
    }
}

impl From<ReferenceCode> for crate::DiagnosticCode {
    fn from(code: ReferenceCode) -> Self {
        DiagnosticCode::Type(TypeCode::Reference(code))
    }
}
