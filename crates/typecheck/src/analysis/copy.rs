//! Copyability via compile-time trait reflection (`Copy.can_copy` from `copy.gin`).

use crate::compile_time_trait::{CompileTimeTraitRegistry, trait_field_for_ty};
use crate::ty::Ty;
use crate::typed::TypedFileAst;
use ast::ConstValue;

/// Extension trait providing `Ty::is_copyable` — trait logic lives in `typecheck`.
pub trait TyCopyExt {
    /// Determine whether this type is copyable via `Copy.can_copy` (blanket or `and has Copy` override).
    fn is_copyable(&self, trait_registry: &CompileTimeTraitRegistry, typed: &TypedFileAst) -> bool;
}

impl TyCopyExt for Ty {
    fn is_copyable(&self, trait_registry: &CompileTimeTraitRegistry, typed: &TypedFileAst) -> bool {
        trait_field_for_ty("Copy", "can_copy", self, typed, trait_registry).is_some_and(|cv| {
            matches!(
                cv,
                ConstValue::Tag { name, .. } if name.as_str() == "True"
            )
        })
    }
}
