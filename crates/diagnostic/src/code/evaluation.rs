use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum EvaluationCode {
    AddressOverflow,
    AddressValue,
    ArityMismatch,
    Cancelled,
    FuelExhausted,
    IndexOutOfBounds,
    IntegerDomainUnresolved,
    InvalidAddress,
    InvalidAddressOffset,
    InvalidAllocation,
    InvalidDeref,
    InvalidDestructure,
    InvalidExpression,
    InvalidIndex,
    InvalidIntrinsic,
    InvalidLoop,
    InvalidOperator,
    InvalidReassignment,
    InvalidSequenceLength,
    InvalidUnionPayload,
    MissingBody,
    RangeOverflow,
    ResidualValue,
    RuntimeCall,
    RuntimeDependency,
    TargetQueryFailed,
    TargetUnavailable,
    UnassignedValue,
    UnfilledHole,
    UnknownField,
    UnknownFunction,
    UnresolvedExpression,
    UnsupportedCast,
    UnsupportedIntrinsic,
}

impl Prefix for EvaluationCode {
    fn prefix(&self) -> &'static str {
        "compile-time"
    }
}

impl Code for EvaluationCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AddressOverflow => "address-overflow",
            Self::AddressValue => "address-value",
            Self::ArityMismatch => "arity-mismatch",
            Self::Cancelled => "cancelled",
            Self::FuelExhausted => "fuel-exhausted",
            Self::IndexOutOfBounds => "index-out-of-bounds",
            Self::IntegerDomainUnresolved => "integer-domain-unresolved",
            Self::InvalidAddress => "invalid-address",
            Self::InvalidAddressOffset => "invalid-address-offset",
            Self::InvalidAllocation => "invalid-allocation",
            Self::InvalidDeref => "invalid-deref",
            Self::InvalidDestructure => "invalid-destructure",
            Self::InvalidExpression => "invalid-expression",
            Self::InvalidIndex => "invalid-index",
            Self::InvalidIntrinsic => "invalid-intrinsic",
            Self::InvalidLoop => "invalid-loop",
            Self::InvalidOperator => "invalid-operator",
            Self::InvalidReassignment => "invalid-reassignment",
            Self::InvalidSequenceLength => "invalid-sequence-length",
            Self::InvalidUnionPayload => "invalid-union-payload",
            Self::MissingBody => "missing-body",
            Self::RangeOverflow => "range-overflow",
            Self::ResidualValue => "residual-value",
            Self::RuntimeCall => "runtime-call",
            Self::RuntimeDependency => "runtime-dependency",
            Self::TargetQueryFailed => "target-query-failed",
            Self::TargetUnavailable => "target-unavailable",
            Self::UnassignedValue => "unassigned-value",
            Self::UnfilledHole => "unfilled-hole",
            Self::UnknownField => "unknown-field",
            Self::UnknownFunction => "unknown-function",
            Self::UnresolvedExpression => "unresolved-expression",
            Self::UnsupportedCast => "unsupported-cast",
            Self::UnsupportedIntrinsic => "unsupported-intrinsic",
        }
    }
}

impl EvaluationCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AddressOverflow,
            Self::AddressValue,
            Self::ArityMismatch,
            Self::Cancelled,
            Self::FuelExhausted,
            Self::IndexOutOfBounds,
            Self::IntegerDomainUnresolved,
            Self::InvalidAddress,
            Self::InvalidAddressOffset,
            Self::InvalidAllocation,
            Self::InvalidDeref,
            Self::InvalidDestructure,
            Self::InvalidExpression,
            Self::InvalidIndex,
            Self::InvalidIntrinsic,
            Self::InvalidLoop,
            Self::InvalidOperator,
            Self::InvalidReassignment,
            Self::InvalidSequenceLength,
            Self::InvalidUnionPayload,
            Self::MissingBody,
            Self::RangeOverflow,
            Self::ResidualValue,
            Self::RuntimeCall,
            Self::RuntimeDependency,
            Self::TargetQueryFailed,
            Self::TargetUnavailable,
            Self::UnassignedValue,
            Self::UnfilledHole,
            Self::UnknownField,
            Self::UnknownFunction,
            Self::UnresolvedExpression,
            Self::UnsupportedCast,
            Self::UnsupportedIntrinsic,
        ]
        .into_iter()
    }
}
