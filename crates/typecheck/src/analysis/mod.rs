//! Analysis — type resolution, inference, flow analysis, and diagnostics.

mod const_expr;
pub mod pattern;
pub use const_expr::{CompTimeEvaluator, ConstEnv};

mod flow;
pub(crate) use flow::{ComparisonOutcome, ScalarComparison};
pub use flow::{FlowContext, VarState};

mod ownership;
mod returns;
pub(crate) use ownership::stage_rematerialize_owned_arguments;
pub use ownership::{TyOwnershipExt, expr_is_rematerializable};
pub use returns::expression_returns;
pub(crate) use returns::kind_returns;
pub(crate) use returns::return_expressions;

mod type_surface;
pub(crate) use type_surface::expr_is_type_surface;
pub use type_surface::{TypeEnv, check_type_application, resolve_name_from_files};

pub(crate) mod validate_consumption;
pub use validate_consumption::stage_validate_consumption;

mod unify;
pub use unify::{unify_type_args, unify_type_args_with_registry};

mod infer;
pub(crate) mod propositions;
pub use infer::{
    LayeredLocals, LocalTypes, TyInfer, TyInferEnv, resolve_parameter_kind_with_subst,
};

mod when_exhaustive;
pub use pattern::pattern_matches_public;
pub(crate) use when_exhaustive::is_patterns_from_arms;
mod constructor_coverage;
pub(crate) use constructor_coverage::patterns_cover_union_variant;
pub use when_exhaustive::{TyWhenExt, when_declare_is_exhaustive, when_is_exhaustive};
