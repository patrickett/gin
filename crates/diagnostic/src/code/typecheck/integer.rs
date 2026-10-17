use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum IntegerCode {
    ArithmeticOverflow,
    BoundedTagRangeConstraint,
    DivisorMayBeZero,
    DomainEmpty,
    LiteralExpectedNonBits,
    LiteralMissingValidity,
    LiteralNegativeRejected,
    LiteralOutOfRange,
    NarrowingFailed,
    NarrowingUnproven,
    OutOfRange,
    RefinementRequiresFiniteBounds,
    RepresentationTooSmall,
    RepresentationWidthInvalid,
    RuntimeWidthUnsupported,
    ShiftCountUnproven,
}

impl Prefix for IntegerCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for IntegerCode {
    fn name(&self) -> &'static str {
        match self {
            Self::ArithmeticOverflow => "integer-arithmetic-overflow",
            Self::BoundedTagRangeConstraint => "in-range-on-bounded-int-tag",
            Self::DivisorMayBeZero => "integer-divisor-may-be-zero",
            Self::DomainEmpty => "integer-domain-empty",
            Self::LiteralExpectedNonBits => "integer-literal-expected-non-bits",
            Self::LiteralMissingValidity => "integer-literal-missing-validity",
            Self::LiteralNegativeRejected => "negative-integer-literal-rejected",
            Self::LiteralOutOfRange => "integer-literal-out-of-range",
            Self::NarrowingFailed => "integer-narrowing-failed",
            Self::NarrowingUnproven => "integer-narrowing-unproven",
            Self::OutOfRange => "out-of-range",
            Self::RefinementRequiresFiniteBounds => "integer-refinement-requires-finite-bounds",
            Self::RepresentationTooSmall => "integer-representation-too-small",
            Self::RepresentationWidthInvalid => "integer-representation-width-invalid",
            Self::RuntimeWidthUnsupported => "integer-runtime-width-unsupported",
            Self::ShiftCountUnproven => "integer-shift-count-unproven",
        }
    }
}

impl IntegerCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::ArithmeticOverflow,
            Self::BoundedTagRangeConstraint,
            Self::DivisorMayBeZero,
            Self::DomainEmpty,
            Self::LiteralExpectedNonBits,
            Self::LiteralMissingValidity,
            Self::LiteralNegativeRejected,
            Self::LiteralOutOfRange,
            Self::NarrowingFailed,
            Self::NarrowingUnproven,
            Self::OutOfRange,
            Self::RefinementRequiresFiniteBounds,
            Self::RepresentationTooSmall,
            Self::RepresentationWidthInvalid,
            Self::RuntimeWidthUnsupported,
            Self::ShiftCountUnproven,
        ]
        .into_iter()
    }
}

impl From<IntegerCode> for crate::DiagnosticCode {
    fn from(code: IntegerCode) -> Self {
        TypeCode::Integer(code).into()
    }
}
