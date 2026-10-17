use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

mod expected;
pub use expected::ExpectedSyntaxCode;
mod removed;
pub use removed::RemovedSyntaxCode;
mod syntax;
pub use syntax::SyntaxCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum ParseCode {
    Expected(#[from] ExpectedSyntaxCode),
    Removed(#[from] RemovedSyntaxCode),
    Syntax(#[from] SyntaxCode),
}

impl Prefix for ParseCode {
    fn prefix(&self) -> &'static str {
        match self {
            Self::Expected(code) => code.prefix(),
            Self::Removed(code) => code.prefix(),
            Self::Syntax(code) => code.prefix(),
        }
    }
}

impl Code for ParseCode {
    fn name(&self) -> &'static str {
        match self {
            Self::Expected(code) => code.name(),
            Self::Removed(code) => code.name(),
            Self::Syntax(code) => code.name(),
        }
    }
}

impl ParseCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        ExpectedSyntaxCode::all()
            .map(Self::Expected)
            .chain(RemovedSyntaxCode::all().map(Self::Removed))
            .chain(SyntaxCode::all().map(Self::Syntax))
    }
}
