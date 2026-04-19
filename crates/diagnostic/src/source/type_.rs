use crate::SpanId;
use crate::{Category, Symptom, SymptomLike};

pub enum TypeSymptom {
    Mismatch,
    UnknownBinding {
        name: String,
    },
    UnknownTag {
        name: String,
    },
    InferenceFailed,
    ConstraintViolation {
        param: String,
        expected: String,
        got: String,
    },
    UnresolvedTypeParam {
        name: String,
    },
    ArityMismatch {
        name: String,
        expected: usize,
        got: usize,
    },
    IndexOutOfBounds {
        index: i128,
        size: usize,
    },
    UnusedBinding {
        name: String,
    },
    NotAVariant {
        name: String,
        union_name: String,
    },
    SelfOutsideMethod,
    EmptyReturn {
        expected_type: String,
    },
}

impl SymptomLike for TypeSymptom {
    fn into_symptom(self, span_id: SpanId) -> Symptom {
        let mut category = Category::Flaw;
        let code: &str;
        let help: Option<String>;
        let message: String;

        match self {
            Self::Mismatch => {
                code = "type-mismatch";
                message = "type mismatch".into();
                help = Some("types do not match".into());
            }
            Self::UnknownBinding { name } => {
                code = "type-unknown-binding";
                message = format!("use of undefined binding `{name}`");
                help = Some("import or define bind before using it".into());
            }
            Self::UnknownTag { name } => {
                code = "type-unknown-tag";
                message = format!("use of undeclared tag `{name}`");
                help = Some("declare the tag before using it".into());
            }
            Self::InferenceFailed => {
                code = "type-inference-failed";
                message = "failed to infer type".into();
                help = Some("could not infer the type".into());
            }
            Self::ConstraintViolation {
                param,
                expected,
                got,
            } => {
                code = "type-constraint-violation";
                message = format!("type parameter `{param}` requires `{expected}`, got `{got}`");
                help = Some(format!(
                    "ensure the type argument for `{param}` satisfies the `{expected}` constraint"
                ));
            }
            Self::UnresolvedTypeParam { name } => {
                code = "type-unresolved-type-param";
                message = format!("unresolved type parameter `{name}`");
                help = Some(format!(
                    "provide a concrete type for `{name}` at the instantiation site"
                ));
            }
            Self::ArityMismatch {
                name,
                expected,
                got,
            } => {
                code = "type-arity-mismatch";
                message = format!("`{name}` expects {expected} type argument(s), got {got}");
                help = Some(format!("provide exactly {expected} type argument(s)"));
            }
            Self::IndexOutOfBounds { index, size } => {
                code = "type-index-out-of-bounds";
                message =
                    format!("index out of bounds: the len is {size} but the index is {index}");
                help = Some(format!("valid indices are 0..{size}"));
            }
            Self::UnusedBinding { name } => {
                category = Category::Help;
                code = "type-unused-binding";
                message = format!("unused binding `{name}`");
                help = Some(
                    "if this is intentional, prefix the name with `_` to suppress this warning"
                        .into(),
                );
            }
            Self::NotAVariant { name, union_name } => {
                code = "type-not-a-variant";
                message = format!("`{name}` is not a variant of `{union_name}`");
                help = Some(format!(
                    "expected one of the variants declared in `{union_name}`"
                ));
            }
            Self::SelfOutsideMethod => {
                code = "type-self-outside-method";
                message = "self used outside method".into();
                help = Some("self can only be used inside methods".into());
            }
            Self::EmptyReturn { expected_type } => {
                code = "type-empty-return";
                message = format!("empty return in function declared to return `{expected_type}`");
                help = Some(format!("expected a variant of `{expected_type}`"));
            }
        }

        Symptom {
            code,
            message,
            help,
            span_id,
            category,
        }
    }
}
