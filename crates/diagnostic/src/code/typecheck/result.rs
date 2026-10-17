use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum ResultCode {
    AnonymousNotNameable,
    FamilyIncompatible,
    FamilyMixedOwners,
    LocalFamilyEscapes,
    NameConflict,
    NamesIncompatible,
    PayloadMismatch,
    ProofConflict,
    ProofFailed,
    ProofUnproven,
    ReturnArity,
    ReturnMismatch,
    ReturnMissing,
    UnknownProjection,
}

impl Prefix for ResultCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for ResultCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AnonymousNotNameable => "anonymous-result-not-nameable",
            Self::FamilyIncompatible => "result-family-incompatible",
            Self::FamilyMixedOwners => "result-family-mixed-owners",
            Self::LocalFamilyEscapes => "local-result-family-escapes",
            Self::NameConflict => "result-name-conflict",
            Self::NamesIncompatible => "incompatible-result-names",
            Self::PayloadMismatch => "result-payload-mismatch",
            Self::ProofConflict => "result-proof-conflict",
            Self::ProofFailed => "result-proof-failed",
            Self::ProofUnproven => "result-proof-unproven",
            Self::ReturnArity => "return-arity",
            Self::ReturnMismatch => "return-mismatch",
            Self::ReturnMissing => "missing-return",
            Self::UnknownProjection => "unknown-result-projection",
        }
    }
}

impl ResultCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AnonymousNotNameable,
            Self::FamilyIncompatible,
            Self::FamilyMixedOwners,
            Self::LocalFamilyEscapes,
            Self::NameConflict,
            Self::NamesIncompatible,
            Self::PayloadMismatch,
            Self::ProofConflict,
            Self::ProofFailed,
            Self::ProofUnproven,
            Self::ReturnArity,
            Self::ReturnMismatch,
            Self::ReturnMissing,
            Self::UnknownProjection,
        ]
        .into_iter()
    }
}

impl From<ResultCode> for crate::DiagnosticCode {
    fn from(code: ResultCode) -> Self {
        TypeCode::Result(code).into()
    }
}
