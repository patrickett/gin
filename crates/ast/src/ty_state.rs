//! Type state machine — tracks the lifecycle of a type slot across analysis passes.
//!
//! Every value-level type slot in the AST starts at `Infer` or `Explicit(TypeExpr)`,
//! transitions to `Resolved(Ty)` during the resolution pass, and may transition
//! to `Narrowed { current, original }` during flow analysis.
//!
//! Progress: Infer ──► Resolved ──► Narrowed
//!           Explicit ──► Resolved ──► Narrowed

use crate::TypeExpr;
use crate::ty::Ty;

/// The state of a type at any value-level slot in the AST.
///
/// Parse time: `Infer` (no annotation) or `Explicit(TypeExpr)` (user wrote a type).
/// After resolution: `Resolved(Ty)`.
/// After flow analysis: `Narrowed { current, original }` for control-flow-refined types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyState {
    /// No annotation — type will be inferred from usage.
    Infer,
    /// User wrote an explicit type annotation. Carries the surface syntax for error messages.
    Explicit(TypeExpr),
    /// Resolved to a concrete type.
    Resolved(Ty),
    /// Control-flow narrowed type (e.g., after `if x is Some(v)`, `x` becomes
    /// `Narrowed { current: Maybe.Some, original: Maybe }` inside the then-branch).
    Narrowed { current: Ty, original: Box<Ty> },
}
