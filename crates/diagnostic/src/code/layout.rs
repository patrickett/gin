use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum LayoutCode {
    AggregateOverflow,
    ArrayOverflow,
    NaturalAlignmentUnrepresentable,
    RawPointerOffset,
    SumOverflow,
    TargetDependent,
    TargetUnsupported,
    ZeroSizedPointeeOffset,
}

impl Prefix for LayoutCode {
    fn prefix(&self) -> &'static str {
        match self {
            Self::TargetDependent => "type",
            _ => "",
        }
    }
}

impl Code for LayoutCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AggregateOverflow => "aggregate-layout-overflow",
            Self::ArrayOverflow => "array-layout-overflow",
            Self::NaturalAlignmentUnrepresentable => {
                "target-layout-natural-alignment-unrepresentable"
            }
            Self::RawPointerOffset => "raw-pointer-offset-layout",
            Self::SumOverflow => "sum-layout-overflow",
            Self::TargetDependent => "target-dependent-layout",
            Self::TargetUnsupported => "unsupported-target-layout",
            Self::ZeroSizedPointeeOffset => "zero-sized-pointee-offset",
        }
    }
}

impl LayoutCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AggregateOverflow,
            Self::ArrayOverflow,
            Self::NaturalAlignmentUnrepresentable,
            Self::RawPointerOffset,
            Self::SumOverflow,
            Self::TargetDependent,
            Self::TargetUnsupported,
            Self::ZeroSizedPointeeOffset,
        ]
        .into_iter()
    }
}
