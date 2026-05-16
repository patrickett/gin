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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

impl TyState {
    /// Returns the concrete type if resolved, stripping any narrowing.
    ///
    /// For [`TyState::Resolved(ty)`] returns `Some(ty)`.
    /// For [`TyState::Narrowed { original, .. }`] returns `Some(original)`.
    /// For [`TyState::Infer`] or [`TyState::Explicit`] returns `None`.
    pub fn resolved_ty(&self) -> Option<&Ty> {
        match self {
            TyState::Resolved(ty) => Some(ty),
            TyState::Narrowed { original, .. } => Some(original),
            TyState::Infer | TyState::Explicit(_) => None,
        }
    }

    /// Returns the current (possibly narrowed) type if resolved.
    ///
    /// For [`TyState::Resolved(ty)`] returns `Some(ty)`.
    /// For [`TyState::Narrowed { current, .. }`] returns `Some(current)`.
    /// For [`TyState::Infer`] or [`TyState::Explicit`] returns `None`.
    pub fn current_ty(&self) -> Option<&Ty> {
        match self {
            TyState::Resolved(ty) => Some(ty),
            TyState::Narrowed { current, .. } => Some(current),
            TyState::Infer | TyState::Explicit(_) => None,
        }
    }

    pub fn is_resolved(&self) -> bool {
        matches!(self, TyState::Resolved(_))
    }

    pub fn is_narrowed(&self) -> bool {
        matches!(self, TyState::Narrowed { .. })
    }
}
