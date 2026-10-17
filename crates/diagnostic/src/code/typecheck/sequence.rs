use super::TypeCode;
use crate::DiagnosticCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum SequenceCode {
    Arity,
    BorrowConflict,
    BuilderRepeatRequiresConstantCount,
    DynamicIndex,
    ElementMismatch,
    ElementsMustUnify,
    IndexOutOfBounds,
    IterationRequiresSequence,
    // type-sequence-literal-requires-context
    LiteralRequiresContext,
    NameMismatch,
    RepeatRequiresContext,
    RepeatSeedNotCopyable,
    UnresolvedAtCodegen,
}

impl Prefix for SequenceCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for SequenceCode {
    fn name(&self) -> &'static str {
        match self {
            Self::Arity => "sequence-arity",
            Self::BorrowConflict => "sequence-borrow-conflict",
            Self::BuilderRepeatRequiresConstantCount => {
                "sequence-builder-repeat-requires-constant-count"
            }
            Self::DynamicIndex => "sequence-dynamic-index",
            Self::ElementMismatch => "sequence-element-mismatch",
            Self::ElementsMustUnify => "sequence-elements-must-unify",
            Self::IndexOutOfBounds => "sequence-index-out-of-bounds",
            Self::IterationRequiresSequence => "for-requires-sequence",
            Self::LiteralRequiresContext => "sequence-literal-requires-context",
            Self::NameMismatch => "sequence-name-mismatch",
            Self::RepeatRequiresContext => "sequence-repeat-requires-context",
            Self::RepeatSeedNotCopyable => "sequence-repeat-seed-not-copyable",
            Self::UnresolvedAtCodegen => "unresolved-sequence-codegen",
        }
    }
}

impl SequenceCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::Arity,
            Self::BorrowConflict,
            Self::BuilderRepeatRequiresConstantCount,
            Self::DynamicIndex,
            Self::ElementMismatch,
            Self::ElementsMustUnify,
            Self::IndexOutOfBounds,
            Self::IterationRequiresSequence,
            Self::LiteralRequiresContext,
            Self::NameMismatch,
            Self::RepeatRequiresContext,
            Self::RepeatSeedNotCopyable,
            Self::UnresolvedAtCodegen,
        ]
        .into_iter()
    }
}

impl From<SequenceCode> for crate::DiagnosticCode {
    fn from(code: SequenceCode) -> Self {
        DiagnosticCode::Type(TypeCode::Sequence(code))
    }
}
