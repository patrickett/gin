use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum PatternCode {
    AlternativeBindingsIncompatible,
    ArmUnreachable,
    ElseMissing,
    ElseNotLast,
    ElseUnreachable,
    MixedWhenForms,
    Proposition,
    Range,
    Wildcard,
    WouldDiscardValue,
}

impl Prefix for PatternCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for PatternCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AlternativeBindingsIncompatible => "incompatible-alternative-bindings",
            Self::ArmUnreachable => "unreachable-when-arm",
            Self::ElseMissing => "missing-else-arm",
            Self::ElseNotLast => "else-not-last",
            Self::ElseUnreachable => "unreachable-else-arm",
            Self::MixedWhenForms => "mixed-when-forms",
            Self::Proposition => "proposition-pattern",
            Self::Range => "range-pattern",
            Self::Wildcard => "wildcard-when-pattern",
            Self::WouldDiscardValue => "pattern-would-discard-value",
        }
    }
}

impl PatternCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AlternativeBindingsIncompatible,
            Self::ArmUnreachable,
            Self::ElseMissing,
            Self::ElseNotLast,
            Self::ElseUnreachable,
            Self::MixedWhenForms,
            Self::Proposition,
            Self::Range,
            Self::Wildcard,
            Self::WouldDiscardValue,
        ]
        .into_iter()
    }
}

impl From<PatternCode> for crate::DiagnosticCode {
    fn from(code: PatternCode) -> Self {
        TypeCode::Pattern(code).into()
    }
}
