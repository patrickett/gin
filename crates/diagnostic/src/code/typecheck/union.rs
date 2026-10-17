use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum UnionCode {
    DiscriminantConversionInvalid,
    DiscriminantDuplicate,
    DiscriminantRequiresUnitVariants,
    DiscriminantsIncomplete,
    ValueIncompatible,
    VariantUnknown,
}

impl Prefix for UnionCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for UnionCode {
    fn name(&self) -> &'static str {
        match self {
            Self::DiscriminantConversionInvalid => "invalid-union-discriminant-conversion",
            Self::DiscriminantDuplicate => "duplicate-union-discriminant",
            Self::DiscriminantRequiresUnitVariants => "discriminant-requires-unit-union",
            Self::DiscriminantsIncomplete => "incomplete-union-discriminants",
            Self::ValueIncompatible => "incompatible-union-value",
            Self::VariantUnknown => "unknown-variant",
        }
    }
}

impl UnionCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::DiscriminantConversionInvalid,
            Self::DiscriminantDuplicate,
            Self::DiscriminantRequiresUnitVariants,
            Self::DiscriminantsIncomplete,
            Self::ValueIncompatible,
            Self::VariantUnknown,
        ]
        .into_iter()
    }
}

impl From<UnionCode> for crate::DiagnosticCode {
    fn from(code: UnionCode) -> Self {
        TypeCode::Union(code).into()
    }
}
