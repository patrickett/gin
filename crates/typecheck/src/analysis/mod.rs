//! Analysis — type resolution, inference, flow analysis, and diagnostics.

mod const_expr;
pub mod pattern;
pub use const_expr::{CompTimeEvaluator, ConstEnv};

mod flow;
pub use flow::{FlowContext, VarState};

mod copy;
pub use copy::TyCopyExt;

mod type_surface;
pub use type_surface::{
    TypeEnv, check_type_application, mangled_fn_call_name, resolve_name_from_files,
};
pub(crate) use type_surface::expr_is_type_surface;

pub(crate) mod validate_consumption;
pub use validate_consumption::stage_validate_consumption;

mod unify;
pub use unify::unify_type_args;

mod infer;
pub use infer::{
    LayeredLocals, LocalTypes, TyInfer, TyInferEnv, resolve_parameter_kind_with_subst,
};

mod when_exhaustive;
pub use pattern::pattern_matches_public;
pub use when_exhaustive::{TyWhenExt, when_declare_is_exhaustive, when_is_exhaustive};
