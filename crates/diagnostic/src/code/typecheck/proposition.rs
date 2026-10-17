use super::TypeCode;
use crate::DiagnosticCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum PropositionCode {
    ArgumentUnproven,
    ConditionRequired,
    Mismatch,
    NumericOperandsRequired,
}

impl Prefix for PropositionCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for PropositionCode {
    fn name(&self) -> &'static str {
        match self {
            Self::ArgumentUnproven => "proposition-argument-unproven",
            Self::ConditionRequired => "condition-requires-proposition",
            Self::Mismatch => "proposition-mismatch",
            Self::NumericOperandsRequired => "numeric-predicate-operands",
        }
    }
}

impl PropositionCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::ArgumentUnproven,
            Self::ConditionRequired,
            Self::Mismatch,
            Self::NumericOperandsRequired,
        ]
        .into_iter()
    }
}

impl From<PropositionCode> for crate::DiagnosticCode {
    fn from(code: PropositionCode) -> Self {
        DiagnosticCode::Type(TypeCode::Proposition(code))
    }
}
