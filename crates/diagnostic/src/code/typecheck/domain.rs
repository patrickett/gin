use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum DomainCode {
    NonInteger,
    Operand,
    RangeArgumentCount,
    Recursion,
}

impl Prefix for DomainCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for DomainCode {
    fn name(&self) -> &'static str {
        match self {
            Self::NonInteger => "non-integer-domain",
            Self::Operand => "domain-operand",
            Self::RangeArgumentCount => "range-argument-count",
            Self::Recursion => "domain-recursion",
        }
    }
}

impl DomainCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::NonInteger,
            Self::Operand,
            Self::RangeArgumentCount,
            Self::Recursion,
        ]
        .into_iter()
    }
}

impl From<DomainCode> for crate::DiagnosticCode {
    fn from(code: DomainCode) -> Self {
        TypeCode::Domain(code).into()
    }
}
