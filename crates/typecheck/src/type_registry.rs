use crate::transform::semantic_reads;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ast::Ty;
use ast::integer::{
    IntegerInterpretation, IntegerRepresentation, IntegerRepresentationRule, IntegerValidity,
    NominalIntegerSemantics,
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
pub struct SequenceLiteralAcceptance {
    pub element_parameter: Intern<String>,
    pub materialization: SequenceLiteralMaterialization,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SequenceLiteralMaterialization {
    FixedArray {
        length_parameter: Intern<String>,
    },
    OwnedBuilder {
        empty_member: Intern<String>,
        push_member: Intern<String>,
    },
}

impl SequenceLiteralAcceptance {
    pub fn fixed_array_length_parameter(&self) -> Option<Intern<String>> {
        match self.materialization {
            SequenceLiteralMaterialization::FixedArray { length_parameter } => {
                Some(length_parameter)
            }
            SequenceLiteralMaterialization::OwnedBuilder { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDeclarationSemantics {
    pub key: DeclarationKey,
    pub display_name: Intern<String>,
    pub parameters: Vec<Intern<String>>,
    pub definition: Ty,
    pub sequence_literal: Option<SequenceLiteralAcceptance>,
    pub private_members: HashMap<Intern<String>, Vec<Vec<Intern<String>>>>,
    pub private_fields: HashSet<Intern<String>>,
    pub field_refinements: HashMap<Intern<String>, ast::PredicateExpr>,
    pub reference: Option<ReferenceSemantics>,
    pub integer: Option<NominalIntegerSemantics>,
    pub discriminant: Option<NominalIntegerSemantics>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrivateMethod {
    pub origin: Option<ast::prelude::FileSemanticOrigin>,
    pub file: u32,
    pub grants: Vec<Vec<Intern<String>>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeRegistry {
    tables: Arc<RegistryTables>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RegistryTables {
    declarations: HashMap<TypeId, TypeDeclarationSemantics>,
    terminal_exits: HashSet<DeclarationKey>,
    private_methods: HashMap<crate::typed::DefId, PrivateMethod>,
    handles: HashMap<DeclarationKey, TypeId>,
    target_dependent_integers: HashSet<DeclarationKey>,
}

impl TypeRegistry {
    pub fn contains_references(&self, ty: &Ty) -> bool {
        let mut pending = vec![ty.clone()];
        let mut visited = HashSet::new();
        for _ in 0..16_384 {
            let Some(ty) = pending.pop() else {
                return false;
            };
            if !visited.insert(ty.clone()) {
                continue;
            }
            if crate::ty::reference_semantics_for_type(&ty, Some(self)).is_some() {
                return true;
            }
            match self.resolved_definition_for_type(&ty) {
                Ty::Record { fields, .. } => pending.extend(fields.into_iter().map(|(_, ty)| *ty)),
                Ty::Array { elem, .. } => pending.push(*elem),
                Ty::Sequence(fields) => pending.extend(fields.into_iter().map(|field| *field.ty)),
                Ty::Union { variants, .. } => pending.extend(
                    variants
                        .into_iter()
                        .flat_map(|variant| variant.fields.into_iter().map(|(_, ty)| *ty)),
                ),
                _ => {}
            }
        }
        true
    }

    pub fn scalar_reference_extent(&self, ty: &Ty) -> Option<u64> {
        let reference = crate::ty::reference_semantics_for_type(ty, Some(self))?;
        match self.resolved_definition_for_type(&reference.pointee) {
            Ty::Ptr { .. } | Ty::Address { .. } | Ty::Array { .. } => None,
            _ => Some(1),
        }
    }

    pub(crate) fn has_owned_storage_fields(&self, ty: &Ty) -> bool {
        matches!(self.resolved_definition_for_type(ty), Ty::Record { fields, .. }
            if fields.iter().any(|(_, field)| matches!(field.as_ref(), Ty::OwnedStorage { .. })))
    }

    pub fn owns_storage_value(&self, ty: &Ty) -> bool {
        self.owns_storage(ty, &mut HashSet::new())
    }

    fn owns_storage(&self, ty: &Ty, seen: &mut HashSet<Ty>) -> bool {
        if !seen.insert(ty.clone())
            || crate::ty::reference_semantics_for_type(ty, Some(self)).is_some()
        {
            return false;
        }
        if matches!(ty, Ty::OwnedStorage { .. }) {
            return true;
        }
        match self.resolved_definition_for_type(ty) {
            Ty::Record { fields, .. } => fields.iter().any(|(_, ty)| self.owns_storage(ty, seen)),
            Ty::Sequence(fields) => fields
                .iter()
                .any(|field| self.owns_storage(&field.ty, seen)),
            Ty::Array { elem, .. } => self.owns_storage(&elem, seen),
            Ty::Union { variants, .. } => variants.iter().any(|variant| {
                variant
                    .fields
                    .iter()
                    .any(|(_, ty)| self.owns_storage(ty, seen))
            }),
            _ => false,
        }
    }

    pub fn storage_requires_borrow_tracking(&self, ty: &Ty) -> bool {
        self.storage_borrows(ty, false, &mut HashSet::new())
    }

    fn storage_borrows(&self, ty: &Ty, contained: bool, seen: &mut HashSet<(Ty, bool)>) -> bool {
        if !seen.insert((ty.clone(), contained)) {
            return false;
        }
        if let Some(reference) = crate::ty::reference_semantics_for_type(ty, Some(self)) {
            return contained || self.storage_borrows(&reference.pointee, false, seen);
        }
        if !contained
            && let Some(instance) = ty.named_instance()
            && self
                .declaration_for_instance(instance)
                .is_some_and(|declaration| {
                    self.has_owned_storage_fields(ty)
                        || (!declaration.private_fields.is_empty()
                            && self.owns_storage(ty, &mut HashSet::new()))
                })
            && instance
                .arguments
                .iter()
                .any(|(_, argument)| match argument {
                    TyArg::Type(ty) => self.storage_borrows(ty, true, &mut HashSet::new()),
                    _ => false,
                })
        {
            return true;
        }
        match self.resolved_definition_for_type(ty) {
            Ty::OwnedStorage { inner } => self.storage_borrows(&inner, true, seen),
            Ty::Record { fields, .. } => fields
                .iter()
                .any(|(_, ty)| self.storage_borrows(ty, contained, seen)),
            Ty::Sequence(fields) => fields
                .iter()
                .any(|field| self.storage_borrows(&field.ty, contained, seen)),
            Ty::Array { elem, .. } => self.storage_borrows(&elem, contained, seen),
            Ty::Union { variants, .. } => variants.iter().any(|variant| {
                variant
                    .fields
                    .iter()
                    .any(|(_, field)| self.storage_borrows(field, contained, seen))
            }),
            _ => false,
        }
    }

    pub fn insert(&mut self, handle: TypeId, semantics: TypeDeclarationSemantics) {
        let tables = Arc::make_mut(&mut self.tables);
        tables.handles.insert(semantics.key.clone(), handle);
        tables.declarations.insert(handle, semantics);
    }

    pub(crate) fn set_terminal_exits(&mut self, exits: impl IntoIterator<Item = DeclarationKey>) {
        let exits = exits.into_iter().collect();
        if self.tables.terminal_exits != exits {
            Arc::make_mut(&mut self.tables).terminal_exits = exits;
        }
    }

    pub(crate) fn has_terminal_exit(&self, ty: &Ty) -> bool {
        semantic_reads::scan();
        self.declaration_for_type(ty)
            .is_some_and(|declaration| self.tables.terminal_exits.contains(&declaration.key))
    }

    pub(crate) fn insert_private_method(&mut self, id: crate::typed::DefId, method: PrivateMethod) {
        Arc::make_mut(&mut self.tables)
            .private_methods
            .insert(id, method);
    }

    pub(crate) fn private_method(&self, id: crate::typed::DefId) -> Option<&PrivateMethod> {
        semantic_reads::scan();
        self.tables.private_methods.get(&id)
    }

    pub fn declaration(&self, handle: TypeId) -> Option<&TypeDeclarationSemantics> {
        semantic_reads::declaration(handle);
        self.tables.declarations.get(&handle)
    }

    /// Return the canonical declaration key for a concrete named type instance.
    ///
    /// This is the stable identity used for declaration-based reflection and
    /// canonical recursion checks.
    pub fn declaration_key(&self, instance: &NamedTypeInstance) -> Option<&DeclarationKey> {
        self.declaration(instance.declaration)
            .map(|declaration| &declaration.key)
    }

    /// Return declaration semantics for a resolved type, including the
    /// compatibility wrappers from parse-level resolution (for example,
    /// `Ty::Ref`). Registry lookup is always by session-local declaration
    /// handle and therefore authoritative for nominal metadata.
    pub fn declaration_for_type(&self, ty: &Ty) -> Option<&TypeDeclarationSemantics> {
        self.declaration_for_instance(ty.named_instance_stripping_reference_wrappers()?)
    }

    pub fn discriminant_semantics_for_type(&self, ty: &Ty) -> Option<&NominalIntegerSemantics> {
        self.declaration_for_type(ty)?.discriminant.as_ref()
    }

    pub fn nominal_integer_semantics_for_type(&self, ty: &Ty) -> Option<&NominalIntegerSemantics> {
        self.declaration_for_type(ty)?.integer.as_ref()
    }

    pub fn integer_validity_for_type<'a>(&'a self, ty: &'a Ty) -> Option<&'a IntegerValidity> {
        self.nominal_integer_semantics_for_type(ty)
            .map(|semantics| &semantics.validity)
            .or_else(|| ty.anonymous_integer_validity())
    }

    pub(crate) fn defer_integer_layout(&mut self, id: TypeId) {
        if let Some(key) = self
            .declaration(id)
            .map(|declaration| declaration.key.clone())
        {
            Arc::make_mut(&mut self.tables)
                .target_dependent_integers
                .insert(key);
        }
    }

    pub fn is_target_dependent_integer(&self, ty: &Ty) -> bool {
        semantic_reads::scan();
        self.declaration_for_type(ty).is_some_and(|declaration| {
            declaration.integer.is_some()
                && self
                    .tables
                    .target_dependent_integers
                    .contains(&declaration.key)
        }) || ty
            .anonymous_integer_validity()
            .is_some_and(|validity| self.is_target_dependent_validity(validity))
    }

    pub(crate) fn is_target_dependent_validity(&self, validity: &IntegerValidity) -> bool {
        semantic_reads::scan();
        if let ast::integer::CanonicalPredicate::Symbolic(expr) = validity.domain().predicate() {
            let mut pending = vec![expr];
            while let Some(expr) = pending.pop() {
                match expr {
                    ast::NormalExpr::TargetQuery { .. } => return true,
                    ast::NormalExpr::Add(lhs, rhs)
                    | ast::NormalExpr::Sub(lhs, rhs)
                    | ast::NormalExpr::Mul(lhs, rhs) => {
                        pending.extend([lhs.as_ref(), rhs.as_ref()])
                    }
                    _ => {}
                }
            }
        }
        self.tables.declarations.values().any(|declaration| {
            self.tables
                .target_dependent_integers
                .contains(&declaration.key)
                && declaration
                    .integer
                    .as_ref()
                    .is_some_and(|integer| &integer.validity == validity)
        })
    }

    pub fn integer_representation_for_type(&self, ty: &Ty) -> Option<IntegerRepresentation> {
        if let Some(semantics) = self.nominal_integer_semantics_for_type(ty) {
            return semantics
                .validity
                .resolve_representation(&semantics.representation_rule)
                .ok();
        }
        let validity = ty.anonymous_integer_validity()?;
        validity
            .resolve_representation(&IntegerRepresentationRule::InferFromValidity)
            .ok()
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

    pub fn sequence_element_type(&self, ty: &Ty) -> Option<Ty> {
        if let Some(element) = ty.sequence_element_type() {
            return Some(element);
        }
        let instance = ty.named_instance_stripping_reference_wrappers()?;
        let acceptance = self
            .declaration_for_instance(instance)?
            .sequence_literal
            .as_ref()?;
        instance.arguments.iter().find_map(|(name, argument)| {
            if *name != acceptance.element_parameter {
                return None;
            }
            match argument {
                TyArg::Type(element) => Some((**element).clone()),
                TyArg::Const(_) => None,
            }
        })
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
        let substitution = crate::subst::DepSubst::from_ty_args(&instance.arguments);
        let mut definition = substitution.apply_to_ty(&declaration.definition);
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
        semantic_reads::handle(key);
        self.tables.handles.get(key).copied()
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
        let tables = Arc::make_mut(&mut self.tables);
        tables
            .terminal_exits
            .extend(other.tables.terminal_exits.iter().cloned());
        tables.private_methods.extend(
            other
                .tables
                .private_methods
                .iter()
                .map(|(id, visibility)| (*id, visibility.clone())),
        );
        tables
            .target_dependent_integers
            .extend(other.tables.target_dependent_integers.iter().cloned());
        for (handle, semantics) in &other.tables.declarations {
            tables.handles.insert(semantics.key.clone(), *handle);
            tables.declarations.insert(*handle, semantics.clone());
        }
    }
}
#[cfg(test)]
#[path = "type_registry/sharing_tests.rs"]
mod sharing_tests;
