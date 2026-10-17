use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum AddressCode {
    ConversionWidthInvalid,
    DereferenceNonAddress,
    PointerOffsetIndexInvalid,
    PointerTypeMismatch,
    RawPointeeMismatch,
    RawPointerPointeeMismatch,
    RawPointerRequiresContext,
    SpaceUnsupported,
    StoreResultInvalid,
    TakingNonPointer,
    UnresolvedAtCodegen,
}

impl Prefix for AddressCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for AddressCode {
    fn name(&self) -> &'static str {
        match self {
            Self::ConversionWidthInvalid => "invalid-address-conversion-width",
            Self::DereferenceNonAddress => "deref-non-address",
            Self::PointerOffsetIndexInvalid => "invalid-pointer-offset-index",
            Self::PointerTypeMismatch => "pointer-type-mismatch",
            Self::RawPointeeMismatch => "raw-address-pointee-mismatch",
            Self::RawPointerPointeeMismatch => "raw-pointer-pointee-mismatch",
            Self::RawPointerRequiresContext => "raw-pointer-requires-expected-type",
            Self::SpaceUnsupported => "unsupported-address-space",
            Self::StoreResultInvalid => "invalid-address-store-result",
            Self::TakingNonPointer => "address-taking-non-pointer",
            Self::UnresolvedAtCodegen => "unresolved-address-codegen",
        }
    }
}

impl AddressCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::ConversionWidthInvalid,
            Self::DereferenceNonAddress,
            Self::PointerOffsetIndexInvalid,
            Self::PointerTypeMismatch,
            Self::RawPointeeMismatch,
            Self::RawPointerPointeeMismatch,
            Self::RawPointerRequiresContext,
            Self::SpaceUnsupported,
            Self::StoreResultInvalid,
            Self::TakingNonPointer,
            Self::UnresolvedAtCodegen,
        ]
        .into_iter()
    }
}

impl From<AddressCode> for crate::DiagnosticCode {
    fn from(code: AddressCode) -> Self {
        TypeCode::Address(code).into()
    }
}
