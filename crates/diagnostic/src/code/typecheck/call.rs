use super::TypeCode;
use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum CallCode {
    ArgumentCount,
    ArgumentModeMismatch,
    ArgumentName,
    AssociatedMethodMissingSelf,
    BeforeInitialization,
    CompileTimeRequiresConstantArguments,
    OverloadAmbiguous,
    OverloadNoMatch,
    SelfParameterTyped,
    // type-invalid-test-callee
    TestCalleeInvalid,
}

impl Prefix for CallCode {
    fn prefix(&self) -> &'static str {
        match self {
            Self::AssociatedMethodMissingSelf => "",
            _ => "type",
        }
    }
}

impl Code for CallCode {
    fn name(&self) -> &'static str {
        match self {
            Self::ArgumentCount => "call-argument-count",
            Self::ArgumentModeMismatch => "argument-mode-mismatch",
            Self::ArgumentName => "call-argument-name",
            Self::AssociatedMethodMissingSelf => "no-self-in-associated-method",
            Self::BeforeInitialization => "call-before-initialization",
            Self::CompileTimeRequiresConstantArguments => "cannot-call-comptime-with-runtime-args",
            Self::OverloadAmbiguous => "ambiguous-overload",
            Self::OverloadNoMatch => "no-matching-overload",
            Self::SelfParameterTyped => "self-param-typed",
            Self::TestCalleeInvalid => "invalid-test-callee",
        }
    }
}

impl CallCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::ArgumentCount,
            Self::ArgumentModeMismatch,
            Self::ArgumentName,
            Self::AssociatedMethodMissingSelf,
            Self::BeforeInitialization,
            Self::CompileTimeRequiresConstantArguments,
            Self::OverloadAmbiguous,
            Self::OverloadNoMatch,
            Self::SelfParameterTyped,
            Self::TestCalleeInvalid,
        ]
        .into_iter()
    }
}

impl From<CallCode> for crate::DiagnosticCode {
    fn from(code: CallCode) -> Self {
        TypeCode::Call(code).into()
    }
}
