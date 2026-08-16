use crate::ty::reference_semantics_for_type;
use crate::type_registry::TypeRegistry;
use ast::{ConstValue, Ty, ty::NamedTypeInstance};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqualityError {
    DifferentTypes,
    UnsupportedType,
}

pub fn validate_with_registry(
    lhs: &Ty,
    rhs: &Ty,
    type_registry: &TypeRegistry,
) -> Result<(), EqualityError> {
    if !compatible_with_registry(lhs, rhs, type_registry) {
        return Err(EqualityError::DifferentTypes);
    }
    if !supported_with_registry(lhs, type_registry) || !supported_with_registry(rhs, type_registry)
    {
        return Err(EqualityError::UnsupportedType);
    }
    Ok(())
}

fn supported_with_registry(ty: &Ty, type_registry: &TypeRegistry) -> bool {
    if let Some(reference) = reference_semantics_for_type(ty, Some(type_registry)) {
        return supported_with_registry(&reference.pointee, type_registry);
    }

    match &type_registry.resolved_definition_for_type(ty) {
        Ty::AnonymousInteger { .. } | Ty::ResultFamily { .. } | Ty::Float { .. } | Ty::Unit => true,
        Ty::Record { fields, .. } => fields
            .iter()
            .all(|(_, field)| supported_with_registry(field, type_registry)),
        Ty::Tuple(fields) => fields
            .iter()
            .all(|field| supported_with_registry(field, type_registry)),
        Ty::Union { variants, .. } => variants.iter().all(|variant| variant.fields.is_empty()),
        Ty::Literal(ConstValue::Int(_) | ConstValue::Float(_)) => true,
        Ty::Literal(ConstValue::Tag { args, .. }) => args.is_empty(),
        Ty::Literal(_) => false,
        Ty::Named { .. } => unreachable!(),
        Ty::Ref { inner, .. } => supported_with_registry(inner, type_registry),
        Ty::UnresolvedLiteral(_)
        | Ty::Opaque(_)
        | Ty::Array { .. }
        | Ty::Ptr { .. }
        | Ty::Address { .. } => false,
    }
}

fn compatible_with_registry(lhs: &Ty, rhs: &Ty, type_registry: &TypeRegistry) -> bool {
    if let (Some(lhs_reference), Some(rhs_reference)) = (
        reference_semantics_for_type(lhs, Some(type_registry)),
        reference_semantics_for_type(rhs, Some(type_registry)),
    ) {
        return lhs_reference.permission == rhs_reference.permission
            && compatible_with_registry(
                &lhs_reference.pointee,
                &rhs_reference.pointee,
                type_registry,
            );
    }

    if !compatible_instances(lhs.named_instance(), rhs.named_instance()) {
        return false;
    }
    let lhs_definition = type_registry.resolved_definition_for_type(lhs);
    let rhs_definition = type_registry.resolved_definition_for_type(rhs);
    match (&lhs_definition, &rhs_definition) {
        (Ty::AnonymousInteger { validity: lhs }, Ty::AnonymousInteger { validity: rhs }) => {
            lhs == rhs
        }
        (Ty::Float { .. }, Ty::Float { .. }) | (Ty::Unit, Ty::Unit) => true,
        (
            Ty::Record {
                name: lhs_name,
                fields: lhs_fields,
                ..
            },
            Ty::Record {
                name: rhs_name,
                fields: rhs_fields,
                ..
            },
        ) => {
            lhs_name == rhs_name
                && lhs_fields.len() == rhs_fields.len()
                && lhs_fields
                    .iter()
                    .zip(rhs_fields)
                    .all(|((lhs_name, lhs), (rhs_name, rhs))| {
                        lhs_name == rhs_name && compatible_with_registry(lhs, rhs, type_registry)
                    })
        }
        (Ty::Tuple(lhs), Ty::Tuple(rhs)) => {
            lhs.len() == rhs.len()
                && lhs
                    .iter()
                    .zip(rhs)
                    .all(|(lhs, rhs)| compatible_with_registry(lhs, rhs, type_registry))
        }
        (Ty::Union { name: lhs_name, .. }, Ty::Union { name: rhs_name, .. }) => {
            lhs_name == rhs_name
        }
        (Ty::Literal(lhs), Ty::Literal(rhs)) => compatible_literals(lhs, rhs),
        (Ty::UnresolvedLiteral(lhs), Ty::UnresolvedLiteral(rhs)) => lhs == rhs,
        (Ty::Opaque(lhs), Ty::Opaque(rhs)) => lhs == rhs,
        (
            Ty::Array {
                elem: lhs,
                size: lhs_size,
            },
            Ty::Array {
                elem: rhs,
                size: rhs_size,
            },
        ) => lhs_size == rhs_size && compatible_with_registry(lhs, rhs, type_registry),
        (Ty::Ptr { inner: lhs }, Ty::Ptr { inner: rhs }) => {
            compatible_with_registry(lhs, rhs, type_registry)
        }
        (Ty::Named { .. }, _) | (_, Ty::Named { .. }) => unreachable!(),
        _ => false,
    }
}

fn compatible_instances(lhs: Option<&NamedTypeInstance>, rhs: Option<&NamedTypeInstance>) -> bool {
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => lhs == rhs,
        (None, None) => true,
        _ => false,
    }
}

fn compatible_literals(lhs: &ConstValue, rhs: &ConstValue) -> bool {
    matches!(
        (lhs, rhs),
        (ConstValue::Int(_), ConstValue::Int(_))
            | (ConstValue::Float(_), ConstValue::Float(_))
            | (ConstValue::String(_), ConstValue::String(_))
            | (ConstValue::Tag { .. }, ConstValue::Tag { .. })
            | (ConstValue::Record { .. }, ConstValue::Record { .. })
            | (ConstValue::List(_), ConstValue::List(_))
    )
}
#[cfg(test)]
#[path = "../tests/equality_tests.rs"]
mod tests;
