use super::TypeCode;
use crate::DiagnosticCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum TargetGroupCode {
    Incompatible,
    InvalidDerivedArgument,
    InvalidProjection,
    OverlappingArguments,
    Unknown,
    UnknownField,
    UnknownVariant,
}

impl Prefix for TargetGroupCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for TargetGroupCode {
    fn name(&self) -> &'static str {
        match self {
            Self::Incompatible => "incompatible-target-group",
            Self::InvalidDerivedArgument => "invalid-derived-target-group-argument",
            Self::InvalidProjection => "invalid-target-group-projection",
            Self::OverlappingArguments => "overlapping-target-group-arguments",
            Self::Unknown => "unknown-target-group",
            Self::UnknownField => "unknown-target-group-field",
            Self::UnknownVariant => "unknown-target-group-variant",
        }
    }
}

impl TargetGroupCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::Incompatible,
            Self::InvalidDerivedArgument,
            Self::InvalidProjection,
            Self::OverlappingArguments,
            Self::Unknown,
            Self::UnknownField,
            Self::UnknownVariant,
        ]
        .into_iter()
    }
}

impl From<TargetGroupCode> for crate::DiagnosticCode {
    fn from(code: TargetGroupCode) -> Self {
        DiagnosticCode::Type(TypeCode::TargetGroup(code))
    }
}
