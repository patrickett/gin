//! Analysis — type resolution, inference, flow analysis, and diagnostics.

use std::collections::HashMap;

use internment::Intern;

use crate::ty::Ty;

mod const_value;

/// Body inference — determines whether each linear parameter is threaded
/// (appears in all return paths) or consumed (never returned).
pub mod infer_convention;
pub use const_value::{Bound, ConstValue, TypeConstraint};

mod flow;
pub use flow::{
    FlowAnalysis, FlowAnalyzer, FlowContext, ImpossibleCheck, IndexOutOfBounds, VarState,
};

mod copy;
pub use copy::is_copyable;

mod resolve;
pub use resolve::{
    is_type_surface, mangled_fn_call_name, resolve_name_from_files, resolve_type_expr_from_map,
    resolve_type_expr_with_subst, substitute_in_ty, typevars_from_receiver,
};

mod desugar_threads;
pub use desugar_threads::stage_desugar_threads;

mod infer;
pub use infer::{
    LayeredLocals, LocalTypes, TyInfer, TyInferEnv, resolve_parameter_kind_with_subst,
};

/// Type alias for variant map entries: (union_name, discriminant, fields)
pub type VariantMapEntry = (Intern<String>, usize, Vec<(Intern<String>, Ty)>);

/// Type alias for the variant map: variant_name -> [(union_name, discriminant, fields)]
pub type VariantMap = HashMap<Intern<String>, Vec<VariantMapEntry>>;

/// Type alias for variant lookup result: (union_name, discriminant, field_slice)
pub type VariantLookupResult<'a> = (Intern<String>, usize, &'a [(Intern<String>, Ty)]);
