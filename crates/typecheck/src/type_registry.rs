use std::collections::{HashMap, HashSet};

use ast::Ty;
use ast::integer::{
    IntegerInterpretation, IntegerRepresentation, IntegerRepresentationRule, IntegerValidity,
    NominalIntegerSemantics, resolve_representation,
};
use ast::ty::{DeclarationKey, NamedTypeInstance, TyArg, TypeId};
use internment::Intern;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferencePermission {
    Observe,
    Mutate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceSemantics {
    pub pointee: Ty,
    pub permission: ReferencePermission,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FixedArrayFormer {
    pub element_parameter: Intern<String>,
    pub length_parameter: Intern<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDeclarationSemantics {
    pub key: DeclarationKey,
    pub display_name: Intern<String>,
    pub parameters: Vec<Intern<String>>,
    pub definition: Ty,
    pub fixed_array: Option<FixedArrayFormer>,
    pub reference: Option<ReferenceSemantics>,
    pub integer: Option<NominalIntegerSemantics>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeRegistry {
    declarations: HashMap<TypeId, TypeDeclarationSemantics>,
    handles: HashMap<DeclarationKey, TypeId>,
}

impl TypeRegistry {
    pub fn insert(&mut self, handle: TypeId, semantics: TypeDeclarationSemantics) {
        self.handles.insert(semantics.key.clone(), handle);
        self.declarations.insert(handle, semantics);
    }

    pub fn declaration(&self, handle: TypeId) -> Option<&TypeDeclarationSemantics> {
        self.declarations.get(&handle)
    }

    /// Return the canonical declaration key for a concrete named type instance.
    ///
    /// This is the stable identity used for declaration-based reflection and
    /// canonical recursion checks.
    pub fn declaration_key(&self, instance: &NamedTypeInstance) -> Option<&DeclarationKey> {
        self.declaration(instance.declaration)
            .map(|declaration| &declaration.key)
    }

    /// Return all declaration handles that currently own metadata for a given
    /// declaration display name.
    pub fn declaration_ids_by_display_name(&self, name: &str) -> Vec<TypeId> {
        self.declarations
            .iter()
            .filter_map(|(handle, declaration)| {
                (declaration.display_name.as_str() == name).then_some(*handle)
            })
            .collect()
    }

    /// Return declaration semantics for a resolved type, including the
    /// compatibility wrappers from parse-level resolution (for example,
    /// `Ty::Ref`). Registry lookup is always by session-local declaration
    /// handle and therefore authoritative for nominal metadata.
    pub fn declaration_for_type(&self, ty: &Ty) -> Option<&TypeDeclarationSemantics> {
        self.declaration_for_instance(ty.named_instance_stripping_reference_wrappers()?)
    }

    pub fn nominal_integer_semantics_for_type(&self, ty: &Ty) -> Option<&NominalIntegerSemantics> {
        self.declaration_for_type(ty)?.integer.as_ref()
    }

    pub fn integer_validity_for_type<'a>(&'a self, ty: &'a Ty) -> Option<&'a IntegerValidity> {
        self.nominal_integer_semantics_for_type(ty)
            .map(|semantics| &semantics.validity)
            .or_else(|| ty.anonymous_integer_validity())
    }

    pub fn integer_representation_for_type(&self, ty: &Ty) -> Option<IntegerRepresentation> {
        if let Some(semantics) = self.nominal_integer_semantics_for_type(ty) {
            return resolve_representation(&semantics.validity, &semantics.representation_rule)
                .ok();
        }
        let validity = ty.anonymous_integer_validity()?;
        resolve_representation(validity, &IntegerRepresentationRule::InferFromValidity).ok()
    }

    pub fn integer_width_for_type(&self, ty: &Ty) -> Option<u32> {
        self.integer_representation_for_type(ty)
            .map(|representation| representation.width().get())
    }

    pub fn nominal_integer_interpretation_for_type(
        &self,
        ty: &Ty,
    ) -> Option<IntegerInterpretation> {
        self.nominal_integer_semantics_for_type(ty)
            .map(|semantics| semantics.interpretation)
    }

    pub fn integer_operation_interpretation_for_type(
        &self,
        ty: &Ty,
    ) -> Option<IntegerInterpretation> {
        self.nominal_integer_interpretation_for_type(ty)
            .or_else(|| ty.anonymous_operation_interpretation())
    }

    /// Resolve reference-former semantics for a concrete named instance. This
    /// follows aliases and applies declaration-argument substitution at each
    /// edge so generic wrappers can continue carrying concrete arguments.
    pub fn reference_former_for_instance(
        &self,
        instance: &NamedTypeInstance,
    ) -> Option<ReferenceSemantics> {
        self.reference_former_for_instance_inner(instance, &mut HashSet::new(), &HashMap::new())
    }

    /// Back-compat convenience for compatibility wrappers that still carry
    /// nominal `Ty::Ref` aliases.
    pub fn reference_former_for_type(&self, ty: &Ty) -> Option<ReferenceSemantics> {
        let instance = ty.named_instance_stripping_reference_wrappers()?;
        self.reference_former_for_instance(instance)
    }

    pub fn declaration_for_instance(
        &self,
        instance: &NamedTypeInstance,
    ) -> Option<&TypeDeclarationSemantics> {
        self.declaration(instance.declaration)
    }

    pub fn resolved_definition_for_type(&self, ty: &Ty) -> Ty {
        self.resolved_definition_for_type_inner(ty, &mut HashSet::new())
    }

    fn resolved_definition_for_type_inner(&self, ty: &Ty, seen: &mut HashSet<TypeId>) -> Ty {
        let Ty::Named { instance, .. } = ty else {
            return ty.clone();
        };
        if !seen.insert(instance.declaration) {
            return ty.clone();
        }
        let Some(declaration) = self.declaration_for_instance(instance) else {
            return ty.clone();
        };
        let substitution = Self::type_substitution_for_instance(instance);
        let mut definition = declaration.definition.substitute(&substitution);
        let is_specialized =
            instance
                .arguments
                .iter()
                .any(|(parameter, argument)| match argument {
                    TyArg::Type(ty) => {
                        !matches!(ty.as_ref(), Ty::Opaque(name) if name == parameter)
                    }
                    TyArg::Const(expr) => expr.to_string() != parameter.as_str(),
                });
        if is_specialized {
            match &mut definition {
                Ty::Record {
                    resolved_params, ..
                }
                | Ty::Union {
                    resolved_params, ..
                } => *resolved_params = Some(instance.arguments.clone()),
                _ => {}
            }
        }
        self.resolved_definition_for_type_inner(&definition, seen)
    }

    pub fn handle(&self, key: &DeclarationKey) -> Option<TypeId> {
        self.handles.get(key).copied()
    }

    fn reference_former_for_instance_inner(
        &self,
        instance: &NamedTypeInstance,
        seen: &mut HashSet<TypeId>,
        active_substitution: &HashMap<Intern<String>, Ty>,
    ) -> Option<ReferenceSemantics> {
        let instance = Self::apply_substitution(instance, active_substitution);
        if seen.contains(&instance.declaration) {
            return None;
        }
        seen.insert(instance.declaration);

        let declaration = self.declaration(instance.declaration)?;
        if let Some(reference) = &declaration.reference {
            let type_substitution = Self::type_substitution_for_instance(&instance);
            return Some(ReferenceSemantics {
                pointee: reference.pointee.substitute(&type_substitution),
                permission: reference.permission.clone(),
            });
        }

        let target = declaration
            .definition
            .named_instance_stripping_reference_wrappers()?;
        if target.declaration == instance.declaration {
            return None;
        }
        let type_substitution = Self::type_substitution_for_instance(&instance);
        self.reference_former_for_instance_inner(target, seen, &type_substitution)
    }

    fn apply_substitution(
        instance: &NamedTypeInstance,
        substitution: &HashMap<Intern<String>, Ty>,
    ) -> NamedTypeInstance {
        if substitution.is_empty() {
            return instance.clone();
        }
        NamedTypeInstance {
            declaration: instance.declaration,
            arguments: instance
                .arguments
                .iter()
                .map(|(parameter, argument)| {
                    let argument = match argument {
                        TyArg::Type(ty) => TyArg::Type(Box::new(ty.substitute(substitution))),
                        TyArg::Const(expr) => TyArg::Const(expr.clone()),
                    };
                    (*parameter, argument)
                })
                .collect(),
        }
    }

    fn type_substitution_for_instance(instance: &NamedTypeInstance) -> HashMap<Intern<String>, Ty> {
        instance
            .arguments
            .iter()
            .filter_map(|(parameter, argument)| match argument {
                TyArg::Type(ty) => Some((*parameter, (**ty).clone())),
                TyArg::Const(_) => None,
            })
            .collect::<HashMap<_, _>>()
    }

    pub fn merge_from(&mut self, other: &TypeRegistry) {
        for (handle, semantics) in &other.declarations {
            self.insert(*handle, semantics.clone());
        }
    }
}
#[cfg(test)]
#[path = "../tests/type_registry_tests.rs"]
mod tests;
