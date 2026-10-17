use super::TypeCode;
use crate::DiagnosticCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum EmissionCode {
    AssemblyInvalid,
    Invalid,
}

impl Prefix for EmissionCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for EmissionCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AssemblyInvalid => "invalid-assembly",
            Self::Invalid => "invalid-emission",
        }
    }
}

impl EmissionCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [Self::AssemblyInvalid, Self::Invalid].into_iter()
    }
}

impl From<EmissionCode> for crate::DiagnosticCode {
    fn from(code: EmissionCode) -> Self {
        DiagnosticCode::Type(TypeCode::Emission(code))
    }
}
