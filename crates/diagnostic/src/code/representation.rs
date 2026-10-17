use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum RepresentationCode {
    ArrayMissing,
    FixedArrayConstructionUnresolved,
    FixedArrayLengthInvalid,
    LiteralUnavailable,
    ProductMissing,
    Recursive,
    SequenceUnavailable,
    SumMissing,
    TextLiteralRequired,
    Unavailable,
    Unsupported,
}

impl Prefix for RepresentationCode {
    fn prefix(&self) -> &'static str {
        match self {
            Self::Unavailable => "type",
            _ => "",
        }
    }
}

impl Code for RepresentationCode {
    fn name(&self) -> &'static str {
        match self {
            Self::ArrayMissing => "array-representation-missing",
            Self::FixedArrayConstructionUnresolved => "unresolved-fixed-array-construction",
            Self::FixedArrayLengthInvalid => "invalid-fixed-array-length",
            Self::LiteralUnavailable => "literal-representation-unavailable",
            Self::ProductMissing => "product-representation-missing",
            Self::Recursive => "recursive-type-representation",
            Self::SequenceUnavailable => "sequence-representation-unavailable",
            Self::SumMissing => "sum-representation-missing",
            Self::TextLiteralRequired => "text-literal-requires-representation",
            Self::Unavailable => "representation-unavailable",
            Self::Unsupported => "unsupported-type-representation",
        }
    }
}

impl RepresentationCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::ArrayMissing,
            Self::FixedArrayConstructionUnresolved,
            Self::FixedArrayLengthInvalid,
            Self::LiteralUnavailable,
            Self::ProductMissing,
            Self::Recursive,
            Self::SequenceUnavailable,
            Self::SumMissing,
            Self::TextLiteralRequired,
            Self::Unavailable,
            Self::Unsupported,
        ]
        .into_iter()
    }
}
