use strum::AsRefStr;

use crate::{Category, DiagnosticLike};

#[derive(Debug, Clone, PartialEq, Eq, AsRefStr)]
pub enum TypeSymptom {
    #[strum(to_string = "type-mismatch")]
    Mismatch,
    #[strum(to_string = "type-unknown-binding")]
    UnknownBinding { name: String },
    #[strum(to_string = "type-unknown-tag")]
    UnknownTag { name: String },
    #[strum(to_string = "type-inference-failed")]
    InferenceFailed,
    #[strum(to_string = "type-constraint-violation")]
    ConstraintViolation {
        param: String,
        expected: String,
        got: String,
    },
    #[strum(to_string = "type-unresolved-type-param")]
    UnresolvedTypeParam { name: String },
    #[strum(to_string = "type-arity-mismatch")]
    ArityMismatch {
        name: String,
        expected: usize,
        got: usize,
    },
    #[strum(to_string = "type-index-out-of-bounds")]
    IndexOutOfBounds { index: i128, size: usize },
    #[strum(to_string = "type-unused-binding")]
    UnusedBinding { name: String },
    #[strum(to_string = "type-not-a-variant")]
    NotAVariant { name: String, union_name: String },
    #[strum(to_string = "type-self-outside-method")]
    SelfOutsideMethod,
    #[strum(to_string = "type-empty-return")]
    EmptyReturn { expected_type: String },
}

impl DiagnosticLike for TypeSymptom {
    fn message(&self) -> String {
        match self {
            Self::Mismatch => "type mismatch".into(),
            Self::UnknownBinding { name } => format!("use of undefined binding `{name}`"),
            Self::UnknownTag { name } => format!("use of undeclared tag `{name}`"),
            Self::InferenceFailed => "failed to infer type".into(),
            Self::ConstraintViolation { param, expected, got } => format!(
                "type parameter `{param}` requires `{expected}`, got `{got}`"
            ),
            Self::UnresolvedTypeParam { name } => format!("unresolved type parameter `{name}`"),
            Self::ArityMismatch { name, expected, got } => format!(
                "`{name}` expects {expected} type argument(s), got {got}"
            ),
            Self::IndexOutOfBounds { index, size } => format!(
                "index out of bounds: the len is {size} but the index is {index}"
            ),
            Self::UnusedBinding { name } => format!("unused binding `{name}`"),
            Self::NotAVariant { name, union_name } => format!(
                "`{name}` is not a variant of `{union_name}`"
            ),
            Self::SelfOutsideMethod => "self used outside method".into(),
            Self::EmptyReturn { expected_type } => format!(
                "empty return in function declared to return `{expected_type}`"
            ),
        }
    }

    fn help(&self) -> Option<String> {
        Some(match self {
            Self::Mismatch => "types do not match".into(),
            Self::UnknownBinding { .. } => "import or define bind before using it".into(),
            Self::UnknownTag { .. } => "declare the tag before using it".into(),
            Self::InferenceFailed => "could not infer the type".into(),
            Self::ConstraintViolation { param, expected, .. } => format!(
                "ensure the type argument for `{param}` satisfies the `{expected}` constraint"
            ),
            Self::UnresolvedTypeParam { name } => format!(
                "provide a concrete type for `{name}` at the instantiation site"
            ),
            Self::ArityMismatch { expected, .. } => format!("provide exactly {expected} type argument(s)"),
            Self::IndexOutOfBounds { size, .. } => format!("valid indices are 0..{size}"),
            Self::UnusedBinding { .. } => "if this is intentional, prefix the name with `_` to suppress this warning".into(),
            Self::NotAVariant { union_name, .. } => format!(
                "expected one of the variants declared in `{union_name}`"
            ),
            Self::SelfOutsideMethod => "self can only be used inside methods".into(),
            Self::EmptyReturn { expected_type } => format!("expected a variant of `{expected_type}`"),
        })
    }

    fn category(&self) -> Category {
        match self {
            Self::UnusedBinding { .. } => Category::Help,
            _ => Category::Flaw,
        }
    }
}
