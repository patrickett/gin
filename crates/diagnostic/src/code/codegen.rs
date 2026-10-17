use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum CodegenCode {
    AddressedPlaceInvalid,
    ArrayConstructorArity,
    ArrayElementTypeMismatch,
    ArrayLengthMismatch,
    ArrayPromotion,
    AssemblyInvalid,
    AssemblyNotChecked,
    BranchBindingMissing,
    BranchJoinInvalid,
    ConstantTypeMismatch,
    ConstructorInvalid,
    DependentArrayLength,
    DomainValue,
    Entry,
    IfJoinUnresolved,
    IfResultUnavailable,
    IntegerConstantDomain,
    IntegerConstantWidth,
    IntegerDomainPatternUnsupported,
    Internal,
    InvalidModule,
    InvalidStackStorage,
    LiteralUnionInvalid,
    LoopBindingMissing,
    LoopPhiInvalid,
    NegationTypeMismatch,
    ObjectSymbols,
    PatternArity,
    PatternParameterUnsupported,
    ReturnLoweringFailed,
    ReturnTypeMismatch,
    SliceRequiresBorrow,
    StringTooLarge,
    SumPayloadArityMismatch,
    TargetQuery,
    VariantDiscriminantInvalid,
    WhenResultUnavailable,
}

impl Prefix for CodegenCode {
    fn prefix(&self) -> &'static str {
        match self {
            Self::SumPayloadArityMismatch => "",
            Self::VariantDiscriminantInvalid => "",
            _ => "codegen",
        }
    }
}

impl Code for CodegenCode {
    fn name(&self) -> &'static str {
        match self {
            Self::AddressedPlaceInvalid => "addressed-place-invalid",
            Self::ArrayConstructorArity => "array-constructor-arity",
            Self::ArrayElementTypeMismatch => "array-element-type-mismatch",
            Self::ArrayLengthMismatch => "array-length-mismatch",
            Self::ArrayPromotion => "array-promotion",
            Self::AssemblyInvalid => "assembly-invalid",
            Self::AssemblyNotChecked => "assembly-not-checked",
            Self::BranchBindingMissing => "branch-binding-missing",
            Self::BranchJoinInvalid => "branch-join-invalid",
            Self::ConstantTypeMismatch => "constant-type-mismatch",
            Self::ConstructorInvalid => "constructor-invalid",
            Self::DependentArrayLength => "dependent-array-length",
            Self::DomainValue => "domain-value",
            Self::Entry => "entry",
            Self::IfJoinUnresolved => "if-join-unresolved",
            Self::IfResultUnavailable => "if-result-unavailable",
            Self::IntegerConstantDomain => "integer-constant-domain",
            Self::IntegerConstantWidth => "integer-constant-width",
            Self::IntegerDomainPatternUnsupported => "integer-domain-pattern-unsupported",
            Self::Internal => "internal",
            Self::InvalidModule => "invalid-module",
            Self::InvalidStackStorage => "invalid-stack-storage",
            Self::LiteralUnionInvalid => "literal-union-invalid",
            Self::LoopBindingMissing => "loop-binding-missing",
            Self::LoopPhiInvalid => "loop-phi-invalid",
            Self::NegationTypeMismatch => "negation-type-mismatch",
            Self::ObjectSymbols => "object-symbols",
            Self::PatternArity => "pattern-arity",
            Self::PatternParameterUnsupported => "pattern-parameter-unsupported",
            Self::ReturnLoweringFailed => "return-lowering-failed",
            Self::ReturnTypeMismatch => "return-type-mismatch",
            Self::SliceRequiresBorrow => "slice-requires-borrow",
            Self::StringTooLarge => "string-too-large",
            Self::SumPayloadArityMismatch => "sum-payload-arity-mismatch",
            Self::TargetQuery => "target-query",
            Self::VariantDiscriminantInvalid => "invalid-variant-discriminant",
            Self::WhenResultUnavailable => "when-result-unavailable",
        }
    }
}

impl CodegenCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::AddressedPlaceInvalid,
            Self::ArrayConstructorArity,
            Self::ArrayElementTypeMismatch,
            Self::ArrayLengthMismatch,
            Self::ArrayPromotion,
            Self::AssemblyInvalid,
            Self::AssemblyNotChecked,
            Self::BranchBindingMissing,
            Self::BranchJoinInvalid,
            Self::ConstantTypeMismatch,
            Self::ConstructorInvalid,
            Self::DependentArrayLength,
            Self::DomainValue,
            Self::Entry,
            Self::IfJoinUnresolved,
            Self::IfResultUnavailable,
            Self::IntegerConstantDomain,
            Self::IntegerConstantWidth,
            Self::IntegerDomainPatternUnsupported,
            Self::Internal,
            Self::InvalidModule,
            Self::InvalidStackStorage,
            Self::LiteralUnionInvalid,
            Self::LoopBindingMissing,
            Self::LoopPhiInvalid,
            Self::NegationTypeMismatch,
            Self::ObjectSymbols,
            Self::PatternArity,
            Self::PatternParameterUnsupported,
            Self::ReturnLoweringFailed,
            Self::ReturnTypeMismatch,
            Self::SliceRequiresBorrow,
            Self::StringTooLarge,
            Self::SumPayloadArityMismatch,
            Self::TargetQuery,
            Self::VariantDiscriminantInvalid,
            Self::WhenResultUnavailable,
        ]
        .into_iter()
    }
}
