//! Staging helpers for availability and future staged-expression work.

mod call;
mod availability;
mod normalize;
mod prepare;

pub use availability::analyze_availability;
pub use call::instantiate_call;
pub use call::CallInstantiation;
pub use normalize::{normalize, require_compile_time, NormalExpr, NormalizeResult};
pub use prepare::prepare_resolved_expressions;

pub type CallableSignatures = std::collections::HashMap<crate::typed::DefId, crate::typed::TypedCallableSignature>;
