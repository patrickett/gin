use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum BindingCode {
    ConstantAfterDeclaration,
    ConstantReassignment,
    Mismatch,
    RebindCrossesCallableBoundary,
    RebindImmutable,
    RebindTargetNotFound,
    RebindViolatesValidity,
    RebindWouldDiscardValue,
    ShadowsVisibleName,
    Unassigned,
    UseBeforeAssignment,
    UseBeforeInitialization,
}

impl Prefix for BindingCode {
    fn prefix(&self) -> &'static str {
        "type"
    }
}

impl Code for BindingCode {
    fn name(&self) -> &'static str {
        match self {
            Self::ConstantAfterDeclaration => "const-bind-after-declare",
            Self::ConstantReassignment => "reassign-constant",
            Self::Mismatch => "binding-mismatch",
            Self::RebindCrossesCallableBoundary => "rebind-crosses-callable-boundary",
            Self::RebindImmutable => "rebind-immutable",
            Self::RebindTargetNotFound => "rebind-target-not-found",
            Self::RebindViolatesValidity => "rebind-violates-validity",
            Self::RebindWouldDiscardValue => "rebind-would-discard-value",
            Self::ShadowsVisibleName => "binding-shadows-visible-name",
            Self::Unassigned => "unassigned-binding",
            Self::UseBeforeAssignment => "use-before-assign",
            Self::UseBeforeInitialization => "use-before-initialization",
        }
    }
}

impl BindingCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::ConstantAfterDeclaration,
            Self::ConstantReassignment,
            Self::Mismatch,
            Self::RebindCrossesCallableBoundary,
            Self::RebindImmutable,
            Self::RebindTargetNotFound,
            Self::RebindViolatesValidity,
            Self::RebindWouldDiscardValue,
            Self::ShadowsVisibleName,
            Self::Unassigned,
            Self::UseBeforeAssignment,
            Self::UseBeforeInitialization,
        ]
        .into_iter()
    }
}

impl From<BindingCode> for crate::DiagnosticCode {
    fn from(code: BindingCode) -> Self {
        TypeCode::Binding(code).into()
    }
}
