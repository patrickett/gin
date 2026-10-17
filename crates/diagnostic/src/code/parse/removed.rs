use super::ParseCode;
use crate::DiagnosticCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum RemovedSyntaxCode {
    AutoAttribute,
    ConsumeArgumentSyntax,
    DefaultAttribute,
    EmptyParens,
    Extern,
    GroupedAttributes,
    HasParens,
    If,
    Intrinsic,
    MarkerSyntax,
    ParameterConventionSyntax,
    PointerFormer,
    PositionalDestructure,
}

impl Prefix for RemovedSyntaxCode {
    fn prefix(&self) -> &'static str {
        "parse"
    }
}

impl Code for RemovedSyntaxCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AutoAttribute => "removed-auto-attribute",
            Self::ConsumeArgumentSyntax => "removed-consume-arg-syntax",
            Self::DefaultAttribute => "removed-default-attribute",
            Self::EmptyParens => "empty-parens-removed",
            Self::Extern => "removed-extern",
            Self::GroupedAttributes => "removed-grouped-attributes",
            Self::HasParens => "removed-has-parens",
            Self::If => "removed-if",
            Self::Intrinsic => "removed-intrinsic",
            Self::MarkerSyntax => "removed-marker-syntax",
            Self::ParameterConventionSyntax => "removed-parameter-convention-syntax",
            Self::PointerFormer => "removed-pointer-former",
            Self::PositionalDestructure => "positional-destructure-removed",
        }
    }
}

impl RemovedSyntaxCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AutoAttribute,
            Self::ConsumeArgumentSyntax,
            Self::DefaultAttribute,
            Self::EmptyParens,
            Self::Extern,
            Self::GroupedAttributes,
            Self::HasParens,
            Self::If,
            Self::Intrinsic,
            Self::MarkerSyntax,
            Self::ParameterConventionSyntax,
            Self::PointerFormer,
            Self::PositionalDestructure,
        ]
        .into_iter()
    }
}

impl From<RemovedSyntaxCode> for crate::DiagnosticCode {
    fn from(code: RemovedSyntaxCode) -> Self {
        DiagnosticCode::Parse(ParseCode::Removed(code))
    }
}
