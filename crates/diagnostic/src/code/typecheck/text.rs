use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum TextCode {
    Mismatch,
    // type-text-literal-requires-context
    RequiresContext,
    TemplatesUnsupported,
    UnresolvedAtCodegen,
}

impl Prefix for TextCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for TextCode {
    fn name(&self) -> &'static str {
        match self {
            Self::Mismatch => "text-literal-mismatch",
            Self::RequiresContext => "text-literal-requires-context",
            Self::TemplatesUnsupported => "template-strings-not-supported",
            Self::UnresolvedAtCodegen => "unresolved-text-literal-codegen",
        }
    }
}

impl TextCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::Mismatch,
            Self::RequiresContext,
            Self::TemplatesUnsupported,
            Self::UnresolvedAtCodegen,
        ]
        .into_iter()
    }
}

impl From<TextCode> for crate::DiagnosticCode {
    fn from(code: TextCode) -> Self {
        TypeCode::Text(code).into()
    }
}
