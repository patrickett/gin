use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum MemberCode {
    ArgumentsRequired,
    Duplicate,
    ExpectedName,
    Missing,
    Private,
    PrivateContext,
    PrivateGrantInvalid,
    ProjectionRequiresAggregate,
    Unknown,
}

impl Prefix for MemberCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for MemberCode {
    fn name(&self) -> &'static str {
        match self {
            Self::ArgumentsRequired => "member-requires-arguments",
            Self::Duplicate => "duplicate-field",
            Self::ExpectedName => "expected-field-name",
            Self::Missing => "missing-field",
            Self::Private => "private-member",
            Self::PrivateContext => "private-member-context",
            Self::PrivateGrantInvalid => "invalid-private-grant",
            Self::ProjectionRequiresAggregate => "projection-requires-aggregate",
            Self::Unknown => "unknown-field",
        }
    }
}

impl MemberCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::ArgumentsRequired,
            Self::Duplicate,
            Self::ExpectedName,
            Self::Missing,
            Self::Private,
            Self::PrivateContext,
            Self::PrivateGrantInvalid,
            Self::ProjectionRequiresAggregate,
            Self::Unknown,
        ]
        .into_iter()
    }
}

impl From<MemberCode> for crate::DiagnosticCode {
    fn from(code: MemberCode) -> Self {
        TypeCode::Member(code).into()
    }
}
