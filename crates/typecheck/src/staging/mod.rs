//! Staging helpers for availability and future staged-expression work.

mod availability;
mod call;
mod normalize;
mod prepare;

pub use availability::analyze_availability;
pub use call::CallInstantiation;
pub use call::instantiate_call;
pub(crate) use call::{argument_order, normalize_inferred_value_type};
pub use normalize::{NormalExpr, NormalizeResult, normalize, normalize_resolved};
pub(crate) use prepare::{has_invalid_compile_time_argument, prepare_resolved_expressions};

pub type CallableSignatures =
    std::collections::HashMap<crate::typed::DefId, crate::typed::TypedCallableSignature>;
