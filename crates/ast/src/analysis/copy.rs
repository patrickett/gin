//! Structural copyability analysis.
//!
//! Determines whether a type can be implicitly copied (e.g. when used as a bare
//! or `own` argument). Copyable types follow value semantics; non-copyable types
//! follow move semantics.

use std::collections::HashSet;

use internment::Intern;

use crate::ty::Ty;

/// Determine whether a type is structurally copyable.
///
/// A type is copyable if:
/// - It is a primitive (int, float, bool, unit)
/// - It is a const union (small integer discriminant)
/// - It is a record/union where all fields are copyable AND the type is not `#[lin]`
/// - It is a tuple where all elements are copyable
///
/// A type is NOT copyable if:
/// - It is a pointer or reference (aliasing concerns)
/// - It is an array (heap-allocated)
/// - It is an opaque/generic type (conservative: assume non-copyable)
/// - It is annotated with `#[not_copy]`
/// - It is annotated with `#[lin]`
pub fn is_copyable(
    ty: &Ty,
    lin_types: &HashSet<Intern<String>>,
    not_copy_types: &HashSet<Intern<String>>,
) -> bool {
    match ty {
        Ty::Int { .. } | Ty::Float { .. } | Ty::Bool | Ty::Unit => true,
        Ty::Ptr { .. } | Ty::Ref { .. } | Ty::Array { .. } => false,
        Ty::Opaque(_) => false,        // conservative for generics
        Ty::ConstUnion { .. } => true, // small integer discriminant

        Ty::Record { name, fields } => {
            if lin_types.contains(name) || not_copy_types.contains(name) {
                return false;
            }
            fields
                .iter()
                .all(|(_, f)| is_copyable(f, lin_types, not_copy_types))
        }
        Ty::Union { name, variants } => {
            if lin_types.contains(name) || not_copy_types.contains(name) {
                return false;
            }
            variants.iter().all(|(_, fields)| {
                fields
                    .iter()
                    .all(|(_, f)| is_copyable(f, lin_types, not_copy_types))
            })
        }
        Ty::Tuple(fields) => fields
            .iter()
            .all(|f| is_copyable(f, lin_types, not_copy_types)),
    }
}

/// Determine whether a type is `#[lin]` (linear).
///
/// A type is linear if it is explicitly annotated with `#[lin]`, or if any
/// of its fields is linear (infectious propagation).
pub fn is_lin_type(
    ty: &Ty,
    lin_types: &HashSet<Intern<String>>,
    visited: &mut HashSet<Intern<String>>,
) -> bool {
    match ty {
        Ty::Record { name, fields } => {
            if lin_types.contains(name) {
                return true;
            }
            if !visited.insert(*name) {
                return false; // already checked, avoid cycles
            }
            fields
                .iter()
                .any(|(_, f)| is_lin_type(f, lin_types, visited))
        }
        Ty::Union { name, variants } => {
            if lin_types.contains(name) {
                return true;
            }
            if !visited.insert(*name) {
                return false; // already checked, avoid cycles
            }
            variants.iter().any(|(_, fields)| {
                fields
                    .iter()
                    .any(|(_, f)| is_lin_type(f, lin_types, visited))
            })
        }
        Ty::Tuple(fields) => fields.iter().any(|f| is_lin_type(f, lin_types, visited)),
        // Primitives, pointers, opaque are never lin
        _ => false,
    }
}
