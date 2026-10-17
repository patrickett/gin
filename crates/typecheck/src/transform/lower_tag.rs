//! Tag/variant resolution helpers used by [`lower_expr_kind`].
//!
//! Resolves tag call expressions to a [`VariantId`] and discriminant,
//! and determines whether a given tag or variant belongs to a union type.
//!
//! [`lower_expr_kind`]: super::lower_kind::lower_expr_kind

use internment::Intern;
use std::collections::HashMap;

use crate::ty::Ty;
use crate::typed::{ExprId, TagId, TypedFileAst, VariantId, VariantMap};

pub(crate) fn type_sum_member(
    expected: Option<&Ty>,
    name: Intern<String>,
    registry: &crate::TypeRegistry,
) -> Option<Ty> {
    let Ty::Union { variants, .. } = registry.resolved_definition_for_type(expected?) else {
        return None;
    };
    variants
        .iter()
        .find(|variant| variant.name == name && variant.kind == ast::ty::UnionVariantKind::Type)
        .and_then(|variant| variant.fields.first().map(|(_, ty)| ty.as_ref().clone()))
}

/// Resolve a tag call to the most likely [`VariantId`] given the available
/// variant map, tag types, and an optional expected return type.
pub(crate) fn resolve_tag_call_variant(
    tag_call: &ast::expr::TagCall,
    variant_map: &VariantMap,
    tag_types: &HashMap<Intern<String>, Ty>,
    expected_ty: Option<&Ty>,
    registry: &crate::TypeRegistry,
) -> VariantId {
    let name = tag_call.name;
    let member = type_sum_member(expected_ty, name, registry);
    let expected_ty = member.as_ref().or(expected_ty);
    if let (Some(expected), Some(declared)) = (expected_ty, tag_types.get(&name))
        && expected
            .named_instance_stripping_reference_wrappers()
            .is_some_and(|instance| {
                declared
                    .named_instance_stripping_reference_wrappers()
                    .is_some_and(|other| instance.declaration == other.declaration)
            })
    {
        return VariantId {
            union: TagId(name),
            name,
        };
    }
    if let Some(Ty::Union {
        name: union,
        variants,
        ..
    }) = expected_ty
        && variants.iter().any(|variant| variant.name == name)
    {
        return VariantId {
            union: TagId(*union),
            name,
        };
    }
    if let Some(qual_path) = &tag_call.qual_path {
        let mut owner = qual_path.value.clone();
        owner.segments.pop();
        let qual_name = Intern::new(owner.to_qualified_string());
        return VariantId {
            union: TagId(qual_name),
            name,
        };
    }
    if let Some(expected) = expected_ty
        && let Some(union_name) = union_name_from_ty(expected)
        && (variant_belongs_to_union(name, union_name, variant_map, tag_types)
            || (matches!(expected, Ty::Opaque(_)) && !tag_types.contains_key(&name)))
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
pub(crate) fn resolve_discriminant(
    variant_id: &VariantId,
    variant_map: &VariantMap,
    expected: Option<&Ty>,
    registry: &crate::TypeRegistry,
) -> usize {
    if let Some(expected) = expected
        && let Ty::Union { variants, .. } = registry.resolved_definition_for_type(expected)
        && let Some(position) = variants
            .iter()
            .position(|variant| variant.name == variant_id.name)
    {
        return position;
    }
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
    registry: &crate::TypeRegistry,
) -> Ty {
    if let Some(member) = type_sum_member(expected_ty, *name, registry) {
        return member.clone();
    }
    if let Some(expected) = expected_ty
        && let Ty::Union { variants, .. } = registry.resolved_definition_for_type(expected)
        && variants.iter().any(|variant| variant.name == *name)
    {
        return expected.clone();
    }
    if let (
        Some(expected @ Ty::Named { instance, .. }),
        Some(Ty::Named {
            instance: declared, ..
        }),
    ) = (expected_ty, tag_types.get(name))
        && instance.declaration == declared.declaration
    {
        return expected.clone();
    }
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
            || (matches!(expected, Ty::Opaque(_)) && !tag_types.contains_key(name)))
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

pub(crate) fn is_record_tag_call(variant_id: &VariantId, ty: &Ty, typed: &TypedFileAst) -> bool {
    let tag_id = TagId(variant_id.name);
    let declared = typed.tag_types.get(&tag_id);
    let matching_declaration = declared.is_some_and(|declared| declared.type_id() == ty.type_id())
        || typed
            .type_registry
            .declaration_for_type(ty)
            .is_some_and(|declaration| declaration.display_name == variant_id.name);
    let receiver = if matching_declaration {
        Some(ty)
    } else {
        declared
    };
    receiver.is_some_and(|receiver| {
        matches!(
            typed.type_registry.resolved_definition_for_type(receiver),
            Ty::Record { .. }
        )
    })
}

pub(crate) fn infer_record_arguments(
    ty: &Ty,
    names: &[Intern<String>],
    args: &[ExprId],
    typed: &TypedFileAst,
) -> Result<Option<Ty>, String> {
    let Ty::Named { instance, .. } = ty else {
        return Ok(None);
    };
    if !instance.arguments.iter().any(|(_, argument)| {
        matches!(argument, ast::TyArg::Type(ty) if matches!(ty.as_ref(), Ty::Opaque(_)))
    }) {
        return Ok(None);
    }
    let Ty::Record { fields, .. } = typed.type_registry.resolved_definition_for_type(ty) else {
        return Ok(None);
    };
    let mut expected = Vec::new();
    let mut actual = Vec::new();
    for (index, argument) in args.iter().enumerate() {
        let Some(name) = names.get(index) else {
            continue;
        };
        let Some((_, field)) = fields.iter().find(|(field, _)| field == name) else {
            continue;
        };
        expected.push(ast::TyArg::Type(field.clone()));
        actual.push(ast::TyArg::Type(Box::new(
            typed.exprs.ty[argument.as_usize()].clone(),
        )));
    }
    crate::analysis::unify_type_args_with_registry(&expected, &actual, Some(&typed.type_registry))
        .map(|substitution| Some(substitution.apply_to_ty(ty)))
}
