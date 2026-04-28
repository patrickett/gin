use strum::AsRefStr;

use crate::SpanId;
use crate::{Category, Diagnostic, DiagnosticCode, DiagnosticLike};

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
    fn into_diagnostic(self, span_id: SpanId) -> Diagnostic {
        let (category, message, help) = match &self {
            Self::Mismatch => (
                Category::Flaw,
                "type mismatch".into(),
                Some("types do not match".into()),
            ),
            Self::UnknownBinding { name } => (
                Category::Flaw,
                format!("use of undefined binding `{name}`"),
                Some("import or define bind before using it".into()),
            ),
            Self::UnknownTag { name } => (
                Category::Flaw,
                format!("use of undeclared tag `{name}`"),
                Some("declare the tag before using it".into()),
            ),
            Self::InferenceFailed => (
                Category::Flaw,
                "failed to infer type".into(),
                Some("could not infer the type".into()),
            ),
            Self::ConstraintViolation {
                param,
                expected,
                got,
            } => (
                Category::Flaw,
                format!("type parameter `{param}` requires `{expected}`, got `{got}`"),
                Some(format!(
                    "ensure the type argument for `{param}` satisfies the `{expected}` constraint"
                )),
            ),
            Self::UnresolvedTypeParam { name } => (
                Category::Flaw,
                format!("unresolved type parameter `{name}`"),
                Some(format!(
                    "provide a concrete type for `{name}` at the instantiation site"
                )),
            ),
            Self::ArityMismatch {
                name,
                expected,
                got,
            } => (
                Category::Flaw,
                format!("`{name}` expects {expected} type argument(s), got {got}"),
                Some(format!("provide exactly {expected} type argument(s)")),
            ),
            Self::IndexOutOfBounds { index, size } => (
                Category::Flaw,
                format!("index out of bounds: the len is {size} but the index is {index}"),
                Some(format!("valid indices are 0..{size}")),
            ),
            Self::UnusedBinding { name } => (
                Category::Help,
                format!("unused binding `{name}`"),
                Some(
                    "if this is intentional, prefix the name with `_` to suppress this warning"
                        .into(),
                ),
            ),
            Self::NotAVariant { name, union_name } => (
                Category::Flaw,
                format!("`{name}` is not a variant of `{union_name}`"),
                Some(format!(
                    "expected one of the variants declared in `{union_name}`"
                )),
            ),
            Self::SelfOutsideMethod => (
                Category::Flaw,
                "self used outside method".into(),
                Some("self can only be used inside methods".into()),
            ),
            Self::EmptyReturn { expected_type } => (
                Category::Flaw,
                format!("empty return in function declared to return `{expected_type}`"),
                Some(format!("expected a variant of `{expected_type}`")),
            ),
        };

        Diagnostic {
            code: DiagnosticCode::Type(self),
            message,
            help,
            span_id,
            category,
        }
    }
}
