//! Analysis — type resolution, inference, flow analysis, and diagnostics.

mod const_expr;
pub mod pattern;
pub use const_expr::{
    check_const_bind_after_declare, check_construction_refinements, eval_compile_time_bind_call,
    eval_compile_time_expr, eval_compile_time_expr_with_env, fold_compile_time_binds,
    validate_compile_time_binds,
};

mod flow;
pub use flow::{
    FlowAnalysis, FlowAnalyzer, FlowContext, ImpossibleCheck, IndexOutOfBounds, VarState,
};

mod copy;
pub use copy::TyCopyExt;

mod type_surface;
pub use type_surface::{
    TypeEnv, check_type_application, mangled_fn_call_name, resolve_name_from_files,
};

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
