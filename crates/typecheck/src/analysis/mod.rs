//! Analysis — type resolution, inference, flow analysis, and diagnostics.

mod const_expr;
pub mod pattern;
pub use const_expr::{
    check_const_bind_after_declare, eval_compile_time_bind_call, eval_compile_time_expr_public,
    eval_compile_time_expr_with_env, fold_compile_time_binds, validate_compile_time_binds,
};

/// Body inference — determines whether each linear parameter is threaded
/// (appears in all return paths) or consumed (never returned).
pub mod infer_convention;

mod flow;
pub use flow::{
    FlowAnalysis, FlowAnalyzer, FlowContext, ImpossibleCheck, IndexOutOfBounds, VarState,
};

mod copy;
pub use copy::TyCopyExt;

mod type_surface;
pub use type_surface::{
    mangled_fn_call_name, resolve_name_from_files, resolve_type_expr_from_map,
    resolve_type_expr_with_subst, resolve_type_expr_with_subst_opts,
};

pub(crate) mod desugar_threads;
pub use desugar_threads::stage_desugar_threads;

mod infer;
pub use infer::{
    LayeredLocals, LocalTypes, TyInfer, TyInferEnv, resolve_parameter_kind_with_subst,
};

mod when_exhaustive;
pub use pattern::pattern_matches_public;
pub use when_exhaustive::{TyWhenExt, when_declare_is_exhaustive, when_is_exhaustive};
