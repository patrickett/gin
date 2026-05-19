//! Copyability analysis — size-based heuristic with explicit opt-out.
//!
//! Copyability is determined in this order:
//!   1. Explicit marker bindings (`and is Copy` / `and is not Copy`) take priority.
//!   2. Otherwise, a type is Copy if its static byte size ≤ 16 bytes.
//!
//! This means small value types are Copy by default, but users can explicitly
//! opt into linear semantics with `and is not Copy` (e.g., `File has (fd Int)
//! and is not Copy`).

use crate::marker::{MarkerRegistry, ty_name};
use crate::ty::{Ty, ty_byte_size_static};
use internment::Intern;

/// Maximum byte size for a type to be implicitly copyable.
const MAX_COPY_BYTES: usize = 16;

/// Determine whether a type is implicitly copyable.
///
/// Priority order:
/// 1. If the `Copy` marker is registered, check explicit `and is Copy` /
///    `and is not Copy` bindings on the type.
/// 2. Otherwise, fall back to the size rule: ≤ 16 bytes → Copy, > 16 → linear.
pub fn is_copyable(ty: &Ty, registry: &MarkerRegistry) -> bool {
    // Check for explicit marker bindings if Copy is a known marker.
    if let Some(type_name) = ty_name(ty)
        && registry.is_recognized(&Intern::from_ref("Copy"))
    {
        // Explicit `and is not Copy` always overrides.
        if registry.has_negative_binding(type_name, &Intern::from_ref("Copy")) {
            return false;
        }
        // Explicit `and is Copy` always overrides.
        if registry.has_positive_binding(type_name, &Intern::from_ref("Copy")) {
            return true;
        }
    }

    // Fall back to size-based rule.
    ty_byte_size_static(ty) <= MAX_COPY_BYTES
}
