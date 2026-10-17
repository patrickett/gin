use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum SymbolCode {
    Unknown,
}

impl Prefix for SymbolCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for SymbolCode {
    fn name(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown-symbol",
        }
    }
}

impl SymbolCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [Self::Unknown].into_iter()
    }
}

impl From<SymbolCode> for crate::DiagnosticCode {
    fn from(code: SymbolCode) -> Self {
        TypeCode::Symbol(code).into()
    }
}
