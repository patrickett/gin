use crate::ty::reference_semantics_for_type;
use crate::type_registry::TypeRegistry;
use ast::{
    ConstValue, Ty,
    ty::{NamedTypeInstance, TypeId},
};

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
    let context = EqualityContext {
        registry: type_registry,
    };
    if !context.compatible(lhs, rhs, &mut Vec::new()) {
        return Err(EqualityError::DifferentTypes);
    }
    if !context.supported(lhs, &mut Vec::new()) || !context.supported(rhs, &mut Vec::new()) {
        return Err(EqualityError::UnsupportedType);
    }
    Ok(())
}

struct EqualityContext<'a> {
    registry: &'a TypeRegistry,
}

impl EqualityContext<'_> {
    fn supported(&self, ty: &Ty, active: &mut Vec<Ty>) -> bool {
        let type_id = declaration(ty);
        if type_id.is_some() {
            if let Some(parent) = active.iter().rfind(|parent| type_id == declaration(parent))
                && !has_type_descendant(parent, ty)
            {
                return false;
            }
            active.push(ty.clone());
        }
        let supported = self.supported_inner(ty, active);
        if type_id.is_some() {
            active.pop();
        }
        supported
    }

    fn supported_inner(&self, ty: &Ty, active: &mut Vec<Ty>) -> bool {
        if let Some(reference) = reference_semantics_for_type(ty, Some(self.registry)) {
            return self.supported(&reference.pointee, active);
        }

        match &self.registry.resolved_definition_for_type(ty) {
            Ty::AnonymousInteger { .. }
            | Ty::Proposition(_)
            | Ty::ResultFamily { .. }
            | Ty::Float { .. }
            | Ty::Unit => true,
            Ty::Record { fields, .. } => fields
                .iter()
                .all(|(_, field)| self.supported(field, active)),
            Ty::Union { variants, .. } => variants.iter().all(|variant| variant.fields.is_empty()),
            Ty::Literal(ConstValue::Int(_) | ConstValue::Float(_)) => true,
            Ty::Literal(ConstValue::Tag { args, .. }) => args.is_empty(),
            Ty::Literal(_) => false,
            Ty::Named { .. } => false,
            Ty::Ref { inner, .. } => self.supported(inner, active),
            Ty::UnresolvedLiteral(_)
            | Ty::UnresolvedSequence { .. }
            | Ty::Opaque(_)
            | Ty::Array { .. }
            | Ty::OwnedStorage { .. }
            | Ty::Ptr { .. }
            | Ty::Address { .. } => false,
            Ty::Sequence(_) => false,
        }
    }

    fn compatible(&self, lhs: &Ty, rhs: &Ty, active: &mut Vec<(Ty, Ty)>) -> bool {
        let references = (
            reference_semantics_for_type(lhs, Some(self.registry)),
            reference_semantics_for_type(rhs, Some(self.registry)),
        );
        if let (Some(lhs), Some(rhs)) = &references {
            if lhs.permission != rhs.permission {
                return false;
            }
        } else if !compatible_instances(lhs.named_instance(), rhs.named_instance()) {
            return false;
        }
        let declarations = (declaration(lhs), declaration(rhs));
        let guarded = declarations.0.is_some() || declarations.1.is_some();
        if guarded {
            if let Some((left, right)) = active
                .iter()
                .rfind(|(left, right)| (declaration(left), declaration(right)) == declarations)
                && !(has_type_descendant(left, lhs) && has_type_descendant(right, rhs))
            {
                return true;
            }
            active.push((lhs.clone(), rhs.clone()));
        }
        let compatible = if let (Some(lhs), Some(rhs)) = references {
            self.compatible(&lhs.pointee, &rhs.pointee, active)
        } else {
            self.compatible_definitions(lhs, rhs, active)
        };
        if guarded {
            active.pop();
        }
        compatible
    }

    fn compatible_definitions(&self, lhs: &Ty, rhs: &Ty, active: &mut Vec<(Ty, Ty)>) -> bool {
        let lhs_definition = self.registry.resolved_definition_for_type(lhs);
        let rhs_definition = self.registry.resolved_definition_for_type(rhs);
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
                    && lhs_fields.iter().zip(rhs_fields).all(
                        |((lhs_name, lhs), (rhs_name, rhs))| {
                            lhs_name == rhs_name && self.compatible(lhs, rhs, active)
                        },
                    )
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
            ) => lhs_size == rhs_size && self.compatible(lhs, rhs, active),
            (Ty::Ptr { inner: lhs }, Ty::Ptr { inner: rhs }) => self.compatible(lhs, rhs, active),
            (Ty::Named { .. }, Ty::Named { .. }) => lhs_definition == rhs_definition,
            _ => false,
        }
    }
}

fn declaration(ty: &Ty) -> Option<TypeId> {
    ty.named_instance().map(|instance| instance.declaration)
}

fn has_type_descendant(parent: &Ty, descendant: &Ty) -> bool {
    parent.any_child(|child| {
        child.ty().is_some_and(|ty| {
            ty == descendant
                || descendant
                    .named_instance()
                    .is_some_and(|instance| ty.named_instance() == Some(instance))
                || has_type_descendant(ty, descendant)
        })
    })
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
            | (ConstValue::Text(_), ConstValue::Text(_))
            | (ConstValue::Tag { .. }, ConstValue::Tag { .. })
            | (ConstValue::Record { .. }, ConstValue::Record { .. })
            | (ConstValue::FixedArray(_), ConstValue::FixedArray(_))
            | (ConstValue::Sequence(_), ConstValue::Sequence(_))
            | (
                ConstValue::SequenceRepeat { .. },
                ConstValue::SequenceRepeat { .. }
            )
    )
}
