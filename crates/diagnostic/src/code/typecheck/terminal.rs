use super::TypeCode;
use crate::DiagnosticCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum TerminalCode {
    Authority,
    EmptyDefault,
    ExitRequired,
    FieldNotConsumed,
    MethodRedundant,
    ValueNotConsumed,
}

impl Prefix for TerminalCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for TerminalCode {
    fn name(&self) -> &'static str {
        match self {
            Self::Authority => "terminal-authority",
            Self::EmptyDefault => "empty-terminal-default",
            Self::ExitRequired => "terminal-exit-required",
            Self::FieldNotConsumed => "terminal-field-not-consumed",
            Self::MethodRedundant => "redundant-terminal-method",
            Self::ValueNotConsumed => "terminal-value-not-consumed",
        }
    }
}

impl TerminalCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::Authority,
            Self::EmptyDefault,
            Self::ExitRequired,
            Self::FieldNotConsumed,
            Self::MethodRedundant,
            Self::ValueNotConsumed,
        ]
        .into_iter()
    }
}

impl From<TerminalCode> for crate::DiagnosticCode {
    fn from(code: TerminalCode) -> Self {
        DiagnosticCode::Type(TypeCode::Terminal(code))
    }
}
