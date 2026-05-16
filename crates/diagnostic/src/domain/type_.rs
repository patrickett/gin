use crate::{Category, DiagnosticLike};

#[derive(Debug, Clone, PartialEq, Eq, strum::AsRefStr)]
#[non_exhaustive]
pub enum TypeSymptom {
    #[strum(serialize = "type-mismatch")]
    Mismatch,
    #[strum(serialize = "type-unknown-binding")]
    UnknownBinding {
        name: String,
        /// Closest in-scope name (imports, functions, tags) within edit distance ≤ 2.
        did_you_mean: Option<String>,
    },
    /// Imported package prefix (or similar) used where a value / callable expression is required.
    #[strum(serialize = "type-not-expr")]
    NotExpr {
        /// Name as written in source (shown in the message as `'name'`).
        name: String,
    },
    #[strum(serialize = "type-unknown-tag")]
    UnknownTag { name: String },
    #[strum(serialize = "type-inference-failed")]
    InferenceFailed,
    #[strum(serialize = "type-constraint-violation")]
    ConstraintViolation {
        param: String,
        expected: String,
        got: String,
    },
    #[strum(serialize = "type-unresolved-type-param")]
    UnresolvedTypeParam { name: String },
    #[strum(serialize = "type-arity-mismatch")]
    ArityMismatch {
        name: String,
        expected: usize,
        got: usize,
    },
    #[strum(serialize = "type-index-out-of-bounds")]
    IndexOutOfBounds { index: i128, size: usize },
    #[strum(serialize = "type-unused-binding")]
    UnusedBinding { name: String },
    #[strum(serialize = "type-not-a-variant")]
    NotAVariant { name: String, union_name: String },
    #[strum(serialize = "type-self-outside-method")]
    SelfOutsideMethod,
    #[strum(serialize = "type-empty-return")]
    EmptyReturn { expected_type: String },
    /// A `when` expression is missing its required `else` clause.
    #[strum(serialize = "type-missing-else-arm")]
    MissingElseArm,
    /// A `when` condition does not resolve to `Bool`.
    #[strum(serialize = "type-condition-not-bool")]
    ConditionNotBool { got: String },
    /// Use of a moved value.
    #[strum(serialize = "type-use-of-moved-value")]
    UseOfMovedValue { name: String },
    /// A `#[lin]` value was not consumed before scope exit.
    #[strum(serialize = "type-lin-value-not-consumed")]
    LinValueNotConsumed { name: String },
    /// Cannot pass a readonly variable as `mut`.
    #[strum(serialize = "type-cannot-pass-readonly-as-mut")]
    CannotPassReadonlyAsMut { name: String },
    /// A positional parameter appears after a parameter with a default value.
    /// Once a default is present, all subsequent parameters must also have defaults
    /// (or be named — see NOTE about named type arguments).
    #[strum(serialize = "type-positional-after-default")]
    PositionalAfterDefault { name: String },
}

impl DiagnosticLike for TypeSymptom {
    fn message(&self) -> String {
        match self {
            Self::Mismatch => "type mismatch".into(),
            Self::UnknownBinding { name, .. } => format!("use of undefined binding `{name}`"),
            Self::NotExpr { name } => format!("'{name}' is not an expression"),
            Self::UnknownTag { name } => format!("use of undeclared tag `{name}`"),
            Self::InferenceFailed => "failed to infer type".into(),
            Self::ConstraintViolation {
                param,
                expected,
                got,
            } => format!("type parameter `{param}` requires `{expected}`, got `{got}`"),
            Self::UnresolvedTypeParam { name } => format!("unresolved type parameter `{name}`"),
            Self::ArityMismatch {
                name,
                expected,
                got,
            } => format!("`{name}` expects {expected} type argument(s), got {got}"),
            Self::IndexOutOfBounds { index, size } => {
                format!("index out of bounds: the len is {size} but the index is {index}")
            }
            Self::UnusedBinding { name } => format!("unused binding `{name}`"),
            Self::NotAVariant { name, union_name } => {
                format!("`{name}` is not a variant of `{union_name}`")
            }
            Self::SelfOutsideMethod => "self used outside method".into(),
            Self::EmptyReturn { expected_type } => {
                format!("empty return in function declared to return `{expected_type}`")
            }
            Self::MissingElseArm => "`when` expression requires an `else` clause".into(),
            Self::ConditionNotBool { got } => {
                format!("`when` condition must be `Bool`, got `{got}`")
            }
            Self::UseOfMovedValue { name } => {
                format!("use of moved value `{name}`")
            }
            Self::LinValueNotConsumed { name } => {
                format!("`#[lin]` value `{name}` was not consumed before scope exit")
            }
            Self::CannotPassReadonlyAsMut { name } => {
                format!("cannot pass `{name}` as `mut` because it is read-only")
            }
            Self::PositionalAfterDefault { name } => {
                format!("positional parameter `{name}` appears after a default parameter")
            }
        }
    }

    fn help_on_span(&self) -> Option<String> {
        match self {
            Self::UnknownBinding { .. } => Some("import or define bind before using it".into()),
            _ => None,
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            Self::UnknownBinding { did_you_mean, .. } => did_you_mean
                .as_ref()
                .map(|m| format!("did you mean `{m}`?")),
            Self::NotExpr { .. } => None,
            Self::Mismatch => Some("types do not match".into()),
            Self::UnknownTag { .. } => Some("declare the tag before using it".into()),
            Self::InferenceFailed => Some("could not infer the type".into()),
            Self::ConstraintViolation {
                param, expected, ..
            } => Some(format!(
                "ensure the type argument for `{param}` satisfies the `{expected}` constraint"
            )),
            Self::UnresolvedTypeParam { name } => Some(format!(
                "provide a concrete type for `{name}` at the instantiation site"
            )),
            Self::ArityMismatch { expected, .. } => {
                Some(format!("provide exactly {expected} type argument(s)"))
            }
            Self::IndexOutOfBounds { size, .. } => Some(format!("valid indices are 0..{size}")),
            Self::UnusedBinding { .. } => Some(
                "if this is intentional, prefix the name with `_` to suppress this warning".into(),
            ),
            Self::NotAVariant { union_name, .. } => Some(format!(
                "expected one of the variants declared in `{union_name}`"
            )),
            Self::SelfOutsideMethod => Some("self can only be used inside methods".into()),
            Self::EmptyReturn { expected_type } => {
                Some(format!("expected a variant of `{expected_type}`"))
            }
            Self::MissingElseArm => Some("add an `else` clause that covers all other cases".into()),
            Self::ConditionNotBool { .. } => Some(
                "the condition must be a `Bool` value (e.g. `x == y` or some `Bool` expression)"
                    .into(),
            ),
            Self::UseOfMovedValue { .. } => {
                Some("value was moved into another owner and cannot be used".into())
            }
            Self::LinValueNotConsumed { .. } => Some(
                "consider passing the value to a consuming function (e.g. `commit(own txn)`)"
                    .into(),
            ),
            Self::CannotPassReadonlyAsMut { .. } => {
                Some("declare the parameter with `mut` or bind with `:` instead of `:=".into())
            },
            Self::PositionalAfterDefault { .. } => Some(
                "all parameters after a default must also have defaults (or use named arguments — see NOTE)".into(),
            ),
        }
    }

    fn category(&self) -> Category {
        match self {
            Self::UnusedBinding { .. } => Category::Help,
            Self::LinValueNotConsumed { .. } => Category::Flaw,
            Self::PositionalAfterDefault { .. } => Category::Flaw,
            _ => Category::Flaw,
        }
    }
}
