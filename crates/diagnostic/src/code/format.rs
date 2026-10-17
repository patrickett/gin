use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum FormatCode {
    DuplicateParameterName,
    FieldShorthand,
    NestedConstruction,
    ReceiverCall,
    RedundantDestructureBinding,
    SingleExpressionBody,
    UnusedGenericTypeName,
}

impl Prefix for FormatCode {
    fn prefix(&self) -> &'static str {
        "fmt"
    }
}

impl Code for FormatCode {
    fn name(&self) -> &'static str {
        match self {
            Self::DuplicateParameterName => "duplicate-parameter-name",
            Self::FieldShorthand => "field-shorthand",
            Self::NestedConstruction => "nested-construction",
            Self::ReceiverCall => "receiver-call",
            Self::RedundantDestructureBinding => "redundant-destructure-binding",
            Self::SingleExpressionBody => "single-expression-body",
            Self::UnusedGenericTypeName => "unused-generic-type-name",
        }
    }
}

impl FormatCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::DuplicateParameterName,
            Self::FieldShorthand,
            Self::NestedConstruction,
            Self::ReceiverCall,
            Self::RedundantDestructureBinding,
            Self::SingleExpressionBody,
            Self::UnusedGenericTypeName,
        ]
        .into_iter()
    }
}
