use crate::{Category, DiagnosticLike};

#[derive(Debug, Clone, PartialEq, Eq, Hash, strum::AsRefStr)]
#[non_exhaustive]
pub enum TypeSymptom {
    #[strum(serialize = "type-mismatch")]
    Mismatch,
    /// Name is not in scope (undefined bind, undeclared tag/trait, missing import).
    #[strum(serialize = "type-unknown-symbol")]
    UnknownSymbol {
        name: String,
        /// Closest in-scope name within edit distance ≤ 2, when applicable.
        did_you_mean: Option<String>,
    },
    /// Imported package prefix (or similar) used where a value / callable expression is required.
    #[strum(serialize = "type-not-expr")]
    NotExpr {
        /// Name as written in source (shown in the message as `'name'`).
        name: String,
    },
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
    /// A `when` expression has an `else` clause but all cases are already covered.
    #[strum(serialize = "type-unreachable-else-arm")]
    UnreachableElseArm,
    /// A `when` condition does not resolve to `Bool`.
    #[strum(serialize = "type-condition-not-bool")]
    ConditionNotBool { got: String },
    /// Use of a moved value.
    #[strum(serialize = "type-use-of-moved-value")]
    UseOfMovedValue { name: String },
    /// A non-Copy value was not consumed before scope exit.
    /// Non-Copy types follow linear rules — they must be explicitly consumed via
    /// `eat` or transferred to another owner.
    #[strum(serialize = "type-lin-value-not-consumed")]
    LinValueNotConsumed {
        name: String,
        /// Inferred consumption paths — methods on this type that consume via `own self`.
        consumption_paths: Vec<String>,
    },
    /// A positional parameter appears after a parameter with a default value.
    /// Once a default is present, all subsequent parameters must also have defaults
    /// (or be named — see NOTE about named type arguments).
    #[strum(serialize = "type-positional-after-default")]
    PositionalAfterDefault { name: String },
    /// A variable was declared with a type but used before being assigned a value.
    #[strum(serialize = "type-use-before-assign")]
    UseBeforeAssign { name: String },
    /// A variable was declared with a type but never assigned a value.
    #[strum(serialize = "type-unassigned-binding")]
    UnassignedBinding { name: String },
    /// A `eat` (consumed) parameter was used as the return expression, which is
    /// not allowed — consumed values must be destroyed within the function, not
    /// returned to the caller.
    #[strum(serialize = "type-return-consumed-param")]
    ReturnConsumedParam { name: String },
    /// `eat` used at call site on an argument whose corresponding parameter
    /// is not declared with `eat`.
    #[strum(serialize = "type-consume-arg-on-bare-param")]
    ConsumeArgOnBareParam { name: String },
    /// `self` parameter has an explicit type annotation that is already known
    /// from the method's receiver type — the annotation is redundant.
    #[strum(serialize = "type-self-param-typed")]
    SelfParamTyped,
    /// A compile-time-known value is outside an `in N...M` / bounded-int parameter type.
    #[strum(serialize = "type-out-of-range")]
    OutOfRange {
        value: i128,
        min: i128,
        max: i128,
    },
    /// `in TinyInt` where `TinyInt` is a bounded-int tag — use `n TinyInt` instead.
    #[strum(serialize = "type-in-range-on-bounded-tag")]
    InRangeOnBoundedIntTag { tag: String },
    /// User attempted to provide a reserved compiler trait (e.g. `Reflectable`).
    #[strum(serialize = "type-reserved-trait-impl")]
    ReservedTraitImpl { trait_name: String },
    /// A `where` trait bound was not satisfied at this site.
    #[strum(serialize = "type-trait-bound-failed")]
    TraitBoundFailed {
        trait_name: String,
        field: String,
        expected: String,
        got: String,
    },
    /// Trait used in `and has Trait(...)` but not imported into this file.
    #[strum(serialize = "type-trait-not-in-scope")]
    TraitNotInScope {
        trait_name: String,
        suggested_import: String,
    },
    /// Reassign or shadow a name introduced with `:=`.
    #[strum(serialize = "type-reassign-constant")]
    ReassignConstant { name: String },
    /// `name Ty` declare followed by `name := expr` in the same scope.
    #[strum(serialize = "type-const-bind-after-declare")]
    ConstBindAfterDeclare { name: String },
    /// Comptime-classified function called from runtime with non-foldable arguments.
    #[strum(serialize = "type-cannot-call-comptime-with-runtime-args")]
    CannotCallComptimeWithRuntimeArgs { fn_name: String, detail: String },
}

impl DiagnosticLike for TypeSymptom {
    fn message(&self) -> String {
        match self {
            Self::Mismatch => "type mismatch".into(),
            Self::UnknownSymbol { name, .. } => format!("use of undefined symbol `{name}`"),
            Self::NotExpr { name } => format!("'{name}' is not an expression"),
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
            Self::UnreachableElseArm => {
                "`else` clause is unreachable — all cases are already covered".into()
            }
            Self::ConditionNotBool { got } => {
                format!("`when` condition must be `Bool`, got `{got}`")
            }
            Self::UseOfMovedValue { name } => {
                format!("use of moved value `{name}`")
            }
            Self::LinValueNotConsumed {
                name,
                consumption_paths,
            } => {
                let mut msg = format!("`{name}` must be consumed before scope exit");
                if !consumption_paths.is_empty() {
                    msg.push_str(&format!(", e.g. with: {}", consumption_paths.join(", ")));
                }
                msg
            }

            Self::UseBeforeAssign { name } => {
                format!("use of unassigned variable `{name}`")
            }
            Self::UnassignedBinding { name } => {
                format!("variable `{name}` was declared but never assigned a value")
            }
            Self::PositionalAfterDefault { name } => {
                format!("positional parameter `{name}` appears after a default parameter")
            }
            Self::ReturnConsumedParam { name } => {
                format!("cannot return consumed parameter `{name}`")
            }
            Self::ConsumeArgOnBareParam { name } => {
                format!("cannot use `eat` on parameter `{name}`: parameter is not consumed")
            }
            Self::SelfParamTyped => {
                "redundant `self` parameter type — the type is already known from the method receiver".into()
            }
            Self::OutOfRange { value, min, max } => {
                format!("value `{value}` is not in range `{min}...{max}` (inclusive)")
            }
            Self::InRangeOnBoundedIntTag { tag } => {
                format!("use `{tag}` without `in` for bounded integer types")
            }
            Self::ReservedTraitImpl { trait_name } => {
                format!("trait `{trait_name}` is provided by the compiler and cannot be implemented manually")
            }
            Self::TraitBoundFailed {
                trait_name,
                field,
                expected,
                got,
            } => format!(
                "`{trait_name}.{field}` bound not satisfied: expected `{expected}`, got `{got}`"
            ),
            Self::ReassignConstant { name } => {
                format!("cannot reassign constant binding `{name}` (bound with `:=`)")
            }
            Self::ConstBindAfterDeclare { name } => format!(
                "constant bind `{name} := …` after `{name} Type` declaration"
            ),
            Self::CannotCallComptimeWithRuntimeArgs { fn_name, detail } => format!(
                "cannot call compile-time function `{fn_name}` here: {detail}"
            ),
            Self::TraitNotInScope {
                trait_name,
                suggested_import,
            } => format!(
                "trait `{trait_name}` is not in scope; import it (e.g. `{suggested_import}`)"
            ),
        }
    }

    fn help_on_span(&self) -> Option<String> {
        match self {
            Self::UnknownSymbol { .. } => {
                Some("import or define the symbol before using it".into())
            }
            _ => None,
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            Self::UnknownSymbol { did_you_mean, .. } => did_you_mean
                .as_ref()
                .map(|m| format!("did you mean `{m}`?")),
            Self::NotExpr { .. } => None,
            Self::Mismatch => Some("types do not match".into()),
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
            Self::UnreachableElseArm => {
                Some("remove the `else` clause — it can never be reached".into())
            }
            Self::ConditionNotBool { .. } => Some(
                "the condition must be a `Bool` value (e.g. `x == y` or some `Bool` expression)"
                    .into(),
            ),
            Self::UseOfMovedValue { .. } => {
                Some("value was moved into another owner and cannot be used".into())
            }
            Self::LinValueNotConsumed { consumption_paths, .. } if !consumption_paths.is_empty() => {
                Some(format!("consume it with one of: {}", consumption_paths.join(", ")))
            }
            Self::LinValueNotConsumed { name, .. } => Some(
                format!("value '{name}' was not consumed via `eat {name}`")
            ),
            Self::UseBeforeAssign { .. } => Some(
                "assign a value to the variable before reading it".into(),
            ),
            Self::UnassignedBinding { .. } => Some(
                "assign a value to the variable with `name: value`".into(),
            ),
            Self::PositionalAfterDefault { .. } => Some(
                "all parameters after a default must also have defaults (or use named arguments — see NOTE)".into(),
            ),
            Self::ReturnConsumedParam { .. } => Some(
                "a `eat` parameter is consumed (destroyed) within the function and cannot be returned".into(),
            ),
            Self::ConsumeArgOnBareParam { .. } => Some(
                "remove the `eat` or declare the parameter with `eat` in the function signature".into(),
            ),
            Self::SelfParamTyped => Some(
                "drop the explicit type — `self` already has the receiver type".into(),
            ),
            Self::OutOfRange { min, max, .. } => {
                Some(format!("use a value between {min} and {max} inclusive"))
            }
            Self::InRangeOnBoundedIntTag { tag } => Some(format!(
                "bounded integer types like `{tag}` use `name {tag}`, not `name in {tag}`"
            )),
            Self::ReservedTraitImpl { .. } => None,
            Self::TraitBoundFailed { .. } => None,
            Self::TraitNotInScope { suggested_import, .. } => {
                Some(format!("add `{suggested_import}` at the top of the file"))
            }
            Self::ReassignConstant { .. } => {
                Some("use `:` for a rebindable binding, or choose a different name".into())
            }
            Self::ConstBindAfterDeclare { .. } => Some(
                "use `name: value` after declare, or a single `name := value` / `name Type: value`"
                    .into(),
            ),
            Self::CannotCallComptimeWithRuntimeArgs { .. } => Some(
                "pass compile-time-known arguments (literals, `:=` constants, or flow constants), or call from a compile-time context"
                    .into(),
            ),
        }
    }

    fn category(&self) -> Category {
        match self {
            Self::SelfParamTyped { .. } => Category::Help,
            Self::UnusedBinding { .. } => Category::Help,
            Self::UnassignedBinding { .. } => Category::Help,
            Self::UnreachableElseArm => Category::Help,
            Self::ConstBindAfterDeclare { .. } => Category::Help,
            Self::LinValueNotConsumed { .. } => Category::Flaw,
            Self::PositionalAfterDefault { .. } => Category::Flaw,
            Self::ReturnConsumedParam { .. } => Category::Flaw,
            Self::ConsumeArgOnBareParam { .. } => Category::Flaw,
            _ => Category::Flaw,
        }
    }
}
