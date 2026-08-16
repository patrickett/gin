//! Tag/variant resolution helpers used by [`lower_expr_kind`].
//!
//! Resolves tag call expressions to a [`VariantId`] and discriminant,
//! and determines whether a given tag or variant belongs to a union type.
//!
//! [`lower_expr_kind`]: super::lower_kind::lower_expr_kind

use internment::Intern;
use std::collections::HashMap;

use crate::ty::Ty;
use crate::typed::{TagId, TypedFileAst, VariantId, VariantMap};

/// Resolve a tag call to the most likely [`VariantId`] given the available
/// variant map, tag types, and an optional expected return type.
pub(crate) fn resolve_tag_call_variant(
    tag_call: &ast::expr::TagCall,
    variant_map: &VariantMap,
    tag_types: &HashMap<Intern<String>, Ty>,
    expected_ty: Option<&Ty>,
) -> VariantId {
    let name = tag_call.name;
    if let Some(qual_path) = &tag_call.qual_path {
        // `qual_path` includes the variant as the last segment.
        // For `Bool.False`: root="Bool", segments=["False"] → union is root.
        // For `Mod.Bool.False`: root="Mod", segments=["Bool","False"] → union is segments[len-2].
        let qual_name = if qual_path.segments.len() > 1 {
            qual_path.segments[qual_path.segments.len() - 2]
        } else {
            qual_path.root
        };
        return VariantId {
            union: TagId(qual_name),
            name,
        };
    }
    if let Some(expected) = expected_ty
        && let Some(union_name) = union_name_from_ty(expected)
        && (variant_belongs_to_union(name, union_name, variant_map, tag_types)
            || matches!(expected, Ty::Opaque(_)))
    {
        return VariantId {
            union: TagId(union_name),
            name,
        };
    }
    if let Some(candidates) = variant_map.get(&name) {
        if let Some(expected) = expected_ty
            && let Some(union_name) = union_name_from_ty(expected)
            && let Some((union, _, _)) = candidates.iter().find(|(u, _, _)| *u == union_name)
        {
            return VariantId {
                union: TagId(*union),
                name,
            };
        }
        if let Some((union_name, _, _)) = candidates.first() {
            return VariantId {
                union: TagId(*union_name),
                name,
            };
        }
    }
    VariantId {
        union: TagId(name),
        name,
    }
}

/// Look up the discriminant (ordinal) of a variant within its union from the
/// variant map.
pub(crate) fn resolve_discriminant(variant_id: &VariantId, variant_map: &VariantMap) -> usize {
    if let Some(candidates) = variant_map.get(&variant_id.name) {
        for (union_name, disc, _) in candidates {
            if *union_name == variant_id.union.0 {
                return *disc;
            }
        }
    }
    0
}

/// Resolve the [`Ty`] of a tag-call expression, using the expected type
/// (if any) to disambiguate variants shared across unions.
pub(crate) fn resolve_tag_call_type(
    name: &Intern<String>,
    tag_types: &HashMap<Intern<String>, Ty>,
    variant_map: &VariantMap,
    expected_ty: Option<&Ty>,
) -> Ty {
    if let Some(expected @ Ty::ResultFamily { alternatives, .. }) = expected_ty
        && alternatives
            .iter()
            .any(|alternative| alternative.label == *name)
    {
        return expected.clone();
    }
    if let Some(expected) = expected_ty
        && let Some(union_name) = union_name_from_ty(expected)
        && (variant_belongs_to_union(*name, union_name, variant_map, tag_types)
            || matches!(expected, Ty::Opaque(_)))
    {
        return expected.clone();
    }
    if let Some(candidates) = variant_map.get(name) {
        if let Some(expected) = expected_ty
            && let Some(union_name) = union_name_from_ty(expected)
            && candidates.iter().any(|(u, _, _)| *u == union_name)
        {
            return expected.clone();
        }
        if let Some((union_name, _, _)) = candidates.first()
            && let Some(ty) = tag_types.get(union_name)
        {
            return ty.clone();
        }
    }
    // Fallback: if the name is a known tag type (e.g. a record like `Coord`),
    // return it directly. This handles shape literal construction like
    // `Coord(x: 1, y: 2)`.
    if let Some(ty) = tag_types.get(name) {
        return ty.clone();
    }
    Ty::Opaque(*name)
}

/// Check whether `variant` is a member of the union identified by `union`
/// by consulting the variant map and tag types.
pub(crate) fn variant_belongs_to_union(
    variant: Intern<String>,
    union: Intern<String>,
    variant_map: &VariantMap,
    tag_types: &HashMap<Intern<String>, Ty>,
) -> bool {
    if let Some(candidates) = variant_map.get(&variant)
        && candidates.iter().any(|(u, _, _)| *u == union)
    {
        return true;
    }
    if let Some(ty) = tag_types.get(&union) {
        if let Ty::Union { variants, .. } = ty {
            return variants.iter().any(|v| v.name == variant);
        }
        if let Some(values) = ty.union_literal_values() {
            return values.iter().any(|cv| cv.display_name() == variant);
        }
    }
    false
}

/// Extract the union name from a type, if it is a union-like type.
pub(crate) fn union_name_from_ty(ty: &Ty) -> Option<Intern<String>> {
    match ty {
        Ty::Union { name, .. } => Some(*name),
        Ty::Named { name, .. } => Some(*name),
        Ty::Opaque(name) => Some(*name),
        _ => None,
    }
}

/// Check whether a [`VariantId`] refers to a record tag (as opposed to a
/// union variant) by looking it up in the typed AST's tag index.
pub(crate) fn is_record_tag_call(variant_id: &VariantId, typed: &TypedFileAst) -> bool {
    let tag_id = TagId(variant_id.name);
    typed.tag_types.get(&tag_id).is_some_and(|ty| {
        matches!(
            typed.type_registry.resolved_definition_for_type(ty),
            Ty::Record { .. }
        )
    })
}
