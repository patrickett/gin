use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum ProofCode {
    AmbiguousMember,
    DependentRefinementFailed,
    DependentRefinementUnproven,
    Impure,
    NonInteger,
    ParameterRefinementFailed,
    ParameterRefinementUnproven,
    PostconditionFailed,
    PostconditionUnproven,
    ProofContractUnknownName,
    TypeMismatch,
    UnknownName,
    UnprovenTerm,
}

impl Prefix for ProofCode {
    fn prefix(&self) -> &'static str {
        match self {
            Self::DependentRefinementFailed => "dep",
            Self::DependentRefinementUnproven => "dep",
            _ => "type",
        }
    }
}

impl Code for ProofCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AmbiguousMember => "constraint-ambiguous-member",
            Self::DependentRefinementFailed => "refinement-failed",
            Self::DependentRefinementUnproven => "refinement-unproven",
            Self::Impure => "constraint-impure",
            Self::NonInteger => "constraint-noninteger",
            Self::ParameterRefinementFailed => "parameter-refinement-failed",
            Self::ParameterRefinementUnproven => "parameter-refinement-unproven",
            Self::PostconditionFailed => "postcondition-failed",
            Self::PostconditionUnproven => "postcondition-unproven",
            Self::ProofContractUnknownName => "proof-contract-unknown-name",
            Self::TypeMismatch => "constraint-type-mismatch",
            Self::UnknownName => "constraint-unknown-name",
            Self::UnprovenTerm => "constraint-unproven-term",
        }
    }
}

impl ProofCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AmbiguousMember,
            Self::DependentRefinementFailed,
            Self::DependentRefinementUnproven,
            Self::Impure,
            Self::NonInteger,
            Self::ParameterRefinementFailed,
            Self::ParameterRefinementUnproven,
            Self::PostconditionFailed,
            Self::PostconditionUnproven,
            Self::ProofContractUnknownName,
            Self::TypeMismatch,
            Self::UnknownName,
            Self::UnprovenTerm,
        ]
        .into_iter()
    }
}

impl From<ProofCode> for crate::DiagnosticCode {
    fn from(code: ProofCode) -> Self {
        TypeCode::Proof(code).into()
    }
}
