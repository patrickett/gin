//! Staging helpers for availability and future staged-expression work.

mod availability;
mod call;
mod normalize;
mod prepare;

pub use availability::analyze_availability;
pub use call::CallInstantiation;
pub use call::instantiate_call;
pub use normalize::{
    NormalExpr, NormalizeResult, normalize, normalize_resolved, require_compile_time,
};
pub use prepare::prepare_resolved_expressions;

pub type CallableSignatures =
    std::collections::HashMap<crate::typed::DefId, crate::typed::TypedCallableSignature>;
