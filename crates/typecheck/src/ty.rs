//! Type representation — re-exported from `ast` for convenience.
//!
//! `Ty` is defined in `ast` because parse-level types like `TyState`
//! and `TypeExpr` reference it. This module re-exports the `ast::ty`
//! types so that consumers of `typecheck` can use `typecheck::ty::Ty`.

use crate::type_registry::{ReferencePermission, ReferenceSemantics, TypeRegistry};
pub use ast::ty::*;

/// Strip one level of safe-reference wrapper and return the inner type with mutability.
pub fn reference_kind(ty: &Ty) -> Option<(&Ty, bool)> {
    match ty {
        Ty::Ref { inner, mutable } => Some((inner, *mutable)),
        _ => None,
    }
}

/// Strip nested safe-reference wrappers to reach the referent type.
pub fn reference_referent(ty: &Ty) -> &Ty {
    match ty {
        Ty::Ref { inner, .. } => reference_referent(inner),
        _ => ty,
    }
}

/// Resolve reference semantics for a type using registry-authenticated metadata
/// when available, while preserving parser-facing `Ty::Ref` compatibility wrappers.
pub fn reference_semantics_for_type(
    ty: &Ty,
    type_registry: Option<&TypeRegistry>,
) -> Option<ReferenceSemantics> {
    if let Some(type_registry) = type_registry {
        if let Some(reference) = ty.named_instance_stripping_reference_wrappers()
            && let Some(reference) = type_registry.reference_former_for_instance(reference)
        {
            return Some(reference);
        }
        if let Some(target) = ty.named_instance()
            && let Some(reference) = type_registry.reference_former_for_instance(target)
        {
            return Some(reference);
        }
    }
    let (inner, mutable) = reference_kind(ty)?;
    Some(ReferenceSemantics {
        pointee: inner.without_reference_wrappers().clone(),
        permission: if mutable {
            ReferencePermission::Mutate
        } else {
            ReferencePermission::Observe
        },
    })
}

/// Return the pointee type for named-reference and compatibility-reference types,
/// when authority can be resolved.
pub fn reference_pointee_for_type(ty: &Ty, type_registry: Option<&TypeRegistry>) -> Option<Ty> {
    reference_semantics_for_type(ty, type_registry).map(|semantics| semantics.pointee)
}
