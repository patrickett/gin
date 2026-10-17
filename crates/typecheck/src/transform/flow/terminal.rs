use super::{WalkExprCtx, is_reference_type};
use crate::analysis::{FlowContext, TyOwnershipExt, VarState};
use crate::typed::{ExprId, PlaceId, PlaceProjection, TargetIndex, TypedExprKind, TypedFileAst};
use ast::{PatternNameExt, Ty};
use diagnostic::code::{MemberCode, OwnershipCode, TerminalCode, TypeCode};
use diagnostic::{Diagnostic, DiagnosticCode};
use std::collections::HashSet;

#[derive(Clone, Copy)]
enum TerminalObligation {
    ModuleAuthority,
    ExplicitOperation,
}

#[derive(Clone, Copy)]
enum TerminalExit {
    Return,
    FunctionEnd,
}

impl TypedFileAst {
    fn emission_construction_places(&self) -> HashSet<PlaceId> {
        let mut places = HashSet::new();
        for kind in &self.exprs.kind {
            let TypedExprKind::Emit {
                specification,
                state,
                ..
            } = kind
            else {
                continue;
            };
            if matches!(state, crate::assembly::AssemblyState::Invalid) {
                continue;
            }
            places.extend(self.exprs.place_of(*specification).flatten());
            if let crate::assembly::AssemblyState::Checked(assembly) = state {
                places.extend(
                    assembly
                        .construction
                        .iter()
                        .filter_map(|expr| self.exprs.place_of(*expr).flatten()),
                );
            }
        }
        places
    }

    pub(in crate::transform) fn borrows_value(&self, value: ExprId) -> bool {
        if self
            .exprs
            .ty_of(value)
            .is_some_and(|ty| is_reference_type(self, ty))
        {
            return true;
        }
        match self.exprs.kind_of(value) {
            Some(TypedExprKind::Deref(base) | TypedExprKind::BufGet { buf: base, .. })
                if self.exprs.ty_of(*base).is_some_and(|ty| {
                    matches!(
                        self.type_registry.resolved_definition_for_type(ty),
                        Ty::Ptr { .. } | Ty::OwnedStorage { .. } | Ty::Address { .. }
                    )
                }) =>
            {
                return self.shared_owned_storage_access(value);
            }
            Some(
                TypedExprKind::Deref(base)
                | TypedExprKind::AggregateGet { base, .. }
                | TypedExprKind::Cast { expr: base, .. }
                | TypedExprKind::UnionPayload { expr: base, .. },
            ) => {
                if self.borrows_value(*base) {
                    return true;
                }
            }
            Some(TypedExprKind::BufGet { buf, .. }) if self.borrows_value(*buf) => {
                return true;
            }
            _ => {}
        }
        let Some(mut place) = self.exprs.place_of(value).flatten() else {
            return false;
        };
        while let Some(parent) = self.places[place.0 as usize].parent {
            place = parent;
        }
        let ast::BinderOwner::Parameter { definition, slot } =
            self.places[place.0 as usize].binder.owner
        else {
            return false;
        };
        self.defs.get(&definition).is_some_and(|bind| {
            matches!(
                bind.param_conventions.get(slot as usize),
                Some(ast::ParamConvention::Observe | ast::ParamConvention::Mutate)
            )
        })
    }

    pub(super) fn borrowed_value_needs_ownership(&self, ty: &Ty) -> bool {
        if is_reference_type(self, ty) {
            return false;
        }
        match self.type_registry.resolved_definition_for_type(ty) {
            Ty::Record { .. } | Ty::Opaque(_) | Ty::OwnedStorage { .. } => true,
            Ty::Union { variants, .. } => variants.iter().any(|variant| !variant.fields.is_empty()),
            Ty::Array { elem, .. } => self.borrowed_value_needs_ownership(&elem),
            Ty::Sequence(fields) => fields
                .iter()
                .any(|field| self.borrowed_value_needs_ownership(&field.ty)),
            _ => false,
        }
    }

    pub(in crate::transform) fn owns_terminal_authority(&self, ty: &Ty) -> bool {
        let Some(declaration) = self.type_registry.declaration_for_type(ty) else {
            return false;
        };
        self.semantic_origin.as_ref().is_some_and(|origin| {
            origin.package == declaration.key.package && origin.module == declaration.key.module
        })
    }

    pub(in crate::transform) fn owns_storage_origin_authority(&self, ty: &Ty) -> bool {
        let ty = crate::ty::reference_pointee_for_type(ty, Some(&self.type_registry))
            .unwrap_or_else(|| ty.clone());
        self.storage_origin_authority(&ty, &mut HashSet::new())
    }

    fn storage_origin_authority(&self, ty: &Ty, seen: &mut HashSet<Ty>) -> bool {
        if !seen.insert(ty.clone()) || is_reference_type(self, ty) {
            return true;
        }
        if self.type_registry.has_owned_storage_fields(ty) {
            return self.owns_terminal_authority(ty);
        }
        match self.type_registry.resolved_definition_for_type(ty) {
            Ty::Record { fields, .. } => fields
                .iter()
                .all(|(_, ty)| self.storage_origin_authority(ty, seen)),
            Ty::Sequence(fields) => fields
                .iter()
                .all(|field| self.storage_origin_authority(&field.ty, seen)),
            Ty::Array { elem, .. } => self.storage_origin_authority(&elem, seen),
            Ty::Union { variants, .. } => variants.iter().all(|variant| {
                variant
                    .fields
                    .iter()
                    .all(|(_, ty)| self.storage_origin_authority(ty, seen))
            }),
            _ => true,
        }
    }

    pub(super) fn requires_terminal_authority(&self, ty: &Ty) -> bool {
        matches!(ty, Ty::Named { .. })
            && matches!(
                self.type_registry.resolved_definition_for_type(ty),
                Ty::Record { .. }
            )
            && !is_reference_type(self, ty)
            && self.type_registry.has_terminal_exit(ty)
    }

    fn pending_terminal_value_in(
        &self,
        ty: &Ty,
        place: Option<PlaceId>,
        flow: &FlowContext,
        seen: &mut HashSet<Ty>,
        obligation: TerminalObligation,
    ) -> bool {
        if place.is_some_and(|place| flow.place_state(place) == Some(VarState::Consumed))
            || is_reference_type(self, ty)
        {
            return false;
        }
        if let Ty::Named { .. } = ty {
            let definition = self.type_registry.resolved_definition_for_type(ty);
            if self.requires_terminal_authority(ty) && !self.owns_terminal_authority(ty) {
                return true;
            }
            if matches!(obligation, TerminalObligation::ExplicitOperation)
                && match &definition {
                    Ty::Record { fields, .. } => {
                        !fields.is_empty() && self.type_registry.has_terminal_exit(ty)
                    }
                    Ty::Union { variants, .. } => variants.iter().any(|variant| {
                        !variant.fields.is_empty()
                            && !place.is_some_and(|place| {
                                flow.excluded_union_variants
                                    .get(&place)
                                    .is_some_and(|excluded| excluded.contains(&variant.name))
                            })
                    }),
                    _ => false,
                }
            {
                return true;
            }
            if !seen.insert(ty.clone()) {
                return false;
            }
            let pending =
                self.pending_terminal_value_in(&definition, place, flow, seen, obligation);
            seen.remove(ty);
            return pending;
        }
        let field_place = |index| {
            let parent = place?;
            self.places
                .iter()
                .position(|candidate| {
                    candidate.parent == Some(parent)
                        && candidate.projection == Some(PlaceProjection::Field(index))
                })
                .map(|index| PlaceId(index as u32))
        };
        match ty {
            Ty::OwnedStorage { .. } => true,
            Ty::Opaque(_) => false,
            Ty::Record { fields, .. } => fields.iter().enumerate().any(|(index, (_, field))| {
                self.pending_terminal_value_in(
                    field,
                    field_place(index),
                    flow,
                    seen,
                    TerminalObligation::ExplicitOperation,
                )
            }),
            Ty::Sequence(fields) => fields.iter().enumerate().any(|(index, field)| {
                self.pending_terminal_value_in(
                    &field.ty,
                    field_place(index),
                    flow,
                    seen,
                    obligation,
                )
            }),
            Ty::Union { variants, .. } => variants.iter().any(|variant| {
                !place.is_some_and(|place| {
                    flow.excluded_union_variants
                        .get(&place)
                        .is_some_and(|excluded| excluded.contains(&variant.name))
                }) && variant.fields.iter().any(|(_, field)| {
                    self.pending_terminal_value_in(field, None, flow, seen, obligation)
                })
            }),
            Ty::Array { elem, size } => {
                if !self.pending_terminal_value_in(elem, None, flow, seen, obligation) {
                    return false;
                }
                let Some(length) = size.as_const_int() else {
                    return true;
                };
                let completed: HashSet<_> = self
                    .places
                    .iter()
                    .enumerate()
                    .filter_map(|(index, child)| {
                        if place.is_none() || child.parent != place {
                            return None;
                        }
                        let Some(PlaceProjection::Item(TargetIndex::Symbolic(item))) =
                            &child.projection
                        else {
                            return None;
                        };
                        let item = item.as_const_int()?;
                        (item >= 0
                            && item < length
                            && !self.pending_terminal_value_in(
                                elem,
                                Some(PlaceId(index as u32)),
                                flow,
                                seen,
                                obligation,
                            ))
                        .then_some(item)
                    })
                    .collect();
                completed.len() as i128 != length
            }
            _ => false,
        }
    }
}

impl WalkExprCtx<'_> {
    pub(super) fn condition_pattern_paths<'a>(
        &self,
        condition: &'a crate::typed::TypedCondition,
        taken: bool,
    ) -> Vec<Vec<super::ConditionPattern<'a>>> {
        let mut paths = super::condition_pattern_paths(condition, taken);
        let mut stable = true;
        condition.visit_subjects(&mut |subject| {
            stable &= !self.changes_record_facts(subject);
        });
        if !stable {
            for path in &mut paths {
                for fact in path {
                    fact.stable = false;
                }
            }
        }
        paths
    }

    pub(super) fn exclude_pattern_variants(
        &mut self,
        subject: ExprId,
        pattern: &ast::Pattern,
        taken: bool,
    ) {
        if self.typed.borrows_value(subject) {
            return;
        }
        let Some(place) = self.typed.exprs.place_of(subject).flatten() else {
            return;
        };
        let Some(ty) = self.typed.exprs.ty_of(subject) else {
            return;
        };
        let Ty::Union { variants, .. } = self.typed.type_registry.resolved_definition_for_type(ty)
        else {
            return;
        };
        let name = pattern.surface_mangle_name();
        let constructor = variants.iter().any(|variant| variant.name.as_str() == name);
        for variant in variants {
            let excluded = if taken {
                constructor && variant.name.as_str() != name
            } else {
                crate::analysis::patterns_cover_union_variant(
                    &[pattern],
                    &variant,
                    Some(&self.typed.type_registry),
                )
            };
            if excluded {
                self.flow_ctx
                    .excluded_union_variants
                    .entry(place)
                    .or_default()
                    .insert(variant.name);
            }
        }
    }

    pub(super) fn forget_union_variant_exclusions(
        &mut self,
        targets: Option<&crate::typed::ReferenceTargetSet>,
    ) {
        self.flow_ctx.excluded_union_variants.retain(|place, _| {
            let Some(targets) = targets else {
                return false;
            };
            let mut root = *place;
            while let Some(parent) = self.typed.places[root.0 as usize].parent {
                root = parent;
            }
            let target = match self.typed.places[root.0 as usize].binder.owner {
                ast::BinderOwner::Parameter { definition, slot } => self
                    .typed
                    .defs
                    .get(&definition)
                    .and_then(|bind| bind.param_groups.get(slot as usize))
                    .copied()
                    .flatten()
                    .map(crate::typed::ReferenceTargetGroup::Param),
                ast::BinderOwner::Expression(_) => {
                    Some(crate::typed::ReferenceTargetGroup::Local(root))
                }
                _ => None,
            };
            target.is_some_and(|target| {
                crate::typed::ReferenceTargetSet::singleton(target)
                    .is_disjoint(targets, &self.flow_ctx.constraint_env)
            })
        });
    }

    pub(super) fn pattern_discards_owned_fields(
        &self,
        pattern: &ast::Pattern,
        subject: ExprId,
    ) -> bool {
        let Some(ty) = self.typed.exprs.ty_of(subject) else {
            return false;
        };
        self.pattern_discards_owned_value(pattern, ty)
    }

    fn pattern_discards_owned_value(&self, pattern: &ast::Pattern, ty: &Ty) -> bool {
        let definition = self.typed.type_registry.resolved_definition_for_type(ty);
        match pattern {
            ast::Pattern::Nominal(name, _) if name.is_pattern_wildcard() => {
                self.pending_terminal_obligation(ty, None)
            }
            ast::Pattern::Generic { name, params, .. } => {
                let fields = match &definition {
                    Ty::Union { variants, .. } => {
                        let Some(variant) = variants.iter().find(|variant| variant.name == *name)
                        else {
                            return false;
                        };
                        &variant.fields
                    }
                    Ty::Record { fields, .. } => fields,
                    _ => return false,
                };
                fields.iter().enumerate().any(|(index, (_, ty))| {
                    params
                        .get(index)
                        .is_none_or(|(binding, _)| binding.is_pattern_wildcard())
                        && self.pending_terminal_obligation(ty, None)
                })
            }
            ast::Pattern::ListCons { head, tail } => {
                let (head_ty, tail_ty) = match &definition {
                    Ty::Array { elem, size } => {
                        let size = size.as_const_int().map_or_else(
                            || {
                                ast::NormalExpr::Sub(
                                    Box::new(size.clone()),
                                    Box::new(ast::NormalExpr::from(1)),
                                )
                            },
                            |length| ast::NormalExpr::from((length - 1).max(0)),
                        );
                        (
                            (**elem).clone(),
                            Ty::Array {
                                elem: elem.clone(),
                                size,
                            },
                        )
                    }
                    Ty::Sequence(fields) => {
                        let Some(first) = fields.first() else {
                            return false;
                        };
                        (
                            (*first.ty).clone(),
                            Ty::Sequence(fields.iter().skip(1).cloned().collect()),
                        )
                    }
                    _ => return false,
                };
                self.pattern_discards_owned_value(&head.value, &head_ty)
                    || self.pattern_discards_owned_value(&tail.value, &tail_ty)
            }
            _ => false,
        }
    }

    pub(super) fn close_pattern_scopes(
        &mut self,
        incoming: &FlowContext,
        branches: &[FlowContext],
    ) {
        for (index, info) in self.typed.places.iter().enumerate() {
            let place = PlaceId(index as u32);
            if info.parent.is_none()
                && matches!(
                    info.binder.owner,
                    ast::BinderOwner::Pattern { .. } | ast::BinderOwner::Expression(_)
                )
                && incoming.place_state(place).is_none()
                && branches
                    .iter()
                    .any(|branch| branch.place_state(place).is_some())
                && branches.iter().all(|branch| {
                    matches!(branch.place_state(place), None | Some(VarState::Consumed))
                        || !self.pending_terminal_obligation_in(&info.ty, Some(place), branch)
                })
            {
                self.flow_ctx.set_place_state(place, VarState::Consumed);
            }
        }
    }

    pub(super) fn pending_terminal_obligation(&self, ty: &Ty, place: Option<PlaceId>) -> bool {
        self.pending_terminal_obligation_in(ty, place, self.flow_ctx)
    }

    pub(super) fn pending_terminal_obligation_in(
        &self,
        ty: &Ty,
        place: Option<PlaceId>,
        flow: &FlowContext,
    ) -> bool {
        let obligation = if self.terminal_receiver.is_some() {
            TerminalObligation::ExplicitOperation
        } else {
            TerminalObligation::ModuleAuthority
        };
        self.typed
            .pending_terminal_value_in(ty, place, flow, &mut HashSet::new(), obligation)
    }

    fn pending_record_fields(
        &self,
        ty: &Ty,
        place: Option<PlaceId>,
        name: String,
    ) -> Vec<(String, Ty)> {
        let mut pending = Vec::new();
        let mut work = vec![(ty.clone(), place, name, HashSet::new())];
        while let Some((ty, place, name, mut ancestors)) = work.pop() {
            if place
                .is_some_and(|place| self.flow_ctx.place_state(place) == Some(VarState::Consumed))
            {
                continue;
            }
            if matches!(&ty, Ty::Named { .. }) && !ancestors.insert(ty.clone()) {
                continue;
            }
            let definition = self.typed.type_registry.resolved_definition_for_type(&ty);
            if !self.typed.requires_terminal_authority(&ty)
                && let Ty::Record { fields, .. } = definition
            {
                for (index, (field, ty)) in fields.into_iter().enumerate().rev() {
                    let child = place.and_then(|parent| {
                        self.typed
                            .places
                            .iter()
                            .position(|candidate| {
                                candidate.parent == Some(parent)
                                    && candidate.projection == Some(PlaceProjection::Field(index))
                            })
                            .map(|index| PlaceId(index as u32))
                    });
                    work.push((*ty, child, format!("{name}.{field}"), ancestors.clone()));
                }
            } else if self.typed.pending_terminal_value_in(
                &ty,
                place,
                self.flow_ctx,
                &mut HashSet::new(),
                TerminalObligation::ExplicitOperation,
            ) {
                pending.push((name, ty));
            }
        }
        pending
    }

    fn terminal_field_obligations(&self) -> Vec<(String, Ty)> {
        let Some(receiver) = self.terminal_receiver else {
            return Vec::new();
        };
        if self.flow_ctx.place_state(receiver) == Some(VarState::Consumed) {
            return Vec::new();
        }
        let definition = self
            .typed
            .type_registry
            .resolved_definition_for_type(&self.typed.places[receiver.0 as usize].ty);
        self.pending_record_fields(&definition, Some(receiver), "self".to_owned())
    }

    fn terminal_field_diagnostic(&self, name: String, ty: &Ty, exit: TerminalExit) -> Diagnostic {
        let type_name = crate::typed::format_ty_for_hover(ty);
        let mut methods: Vec<_> = self
            .callable_signatures
            .iter()
            .filter(|(id, _)| self.typed.can_access_method(**id))
            .filter_map(|(id, signature)| {
                signature
                    .params
                    .iter()
                    .zip(&signature.param_conventions)
                    .any(|((name, param), convention)| {
                        name.as_str() == "self"
                            && param == ty
                            && *convention == ast::ParamConvention::Terminal
                    })
                    .then(|| format!("`{}`", id.0.as_str().rsplit('.').next().unwrap()))
            })
            .collect();
        methods.sort();
        methods.dedup();
        let operation = if methods.is_empty() {
            "explicitly pass it to a terminal operation".to_owned()
        } else {
            format!("call its {} terminal method", methods.join(" or "))
        };
        let (label, deadline) = match exit {
            TerminalExit::Return => (
                format!("returning here leaves `{name}` unconsumed"),
                "before returning",
            ),
            TerminalExit::FunctionEnd => (
                format!("function ends here with `{name}` unconsumed"),
                "before the function ends",
            ),
        };
        // type-terminal-field-not-consumed
        Diagnostic::new(
            TerminalCode::FieldNotConsumed,
            format!("`{name}` is still pending"),
        )
        .with_help_on_span(label)
        .with_help(format!(
            "`{name}` contains a `{type_name}`; {operation}, or transfer ownership {deadline}"
        ))
        .with_arg("name", name)
        .with_arg("type_name", type_name)
    }

    fn pending_value_diagnostics(&self, exit: TerminalExit) -> Vec<Diagnostic> {
        let construction = self.typed.emission_construction_places();
        let pending: Vec<_> = self
            .typed
            .places
            .iter()
            .enumerate()
            .filter_map(|(index, place)| {
                let id = PlaceId(index as u32);
                if place.parent.is_some()
                    || !self.in_terminal_scope(id)
                    || self.terminal_receiver == Some(id)
                    || construction.contains(&id)
                    || !matches!(
                        self.flow_ctx.place_state(id),
                        Some(VarState::Alive | VarState::MaybeConsumed)
                    )
                    || !self.pending_terminal_obligation(&place.ty, Some(id))
                {
                    return None;
                }
                Some((id, place.name, place.ty.clone()))
            })
            .collect();
        let mut diagnostics = Vec::new();
        for (place, name, ty) in pending {
            if !self.typed.requires_terminal_authority(&ty)
                && matches!(
                    self.typed.type_registry.resolved_definition_for_type(&ty),
                    Ty::Record { .. }
                )
            {
                diagnostics.extend(
                    self.pending_record_fields(&ty, Some(place), name.to_string())
                        .into_iter()
                        .map(|(name, ty)| self.terminal_field_diagnostic(name, &ty, exit)),
                );
            } else {
                // type-terminal-value-not-consumed
                diagnostics.push(Diagnostic::new(
                    TerminalCode::ValueNotConsumed,
                    format!("`{name}` must be transferred or passed to a terminal operation from its defining module"),
                ).with_arg("name", name.as_str()));
            }
        }
        diagnostics
    }

    fn incomplete_borrow_diagnostics(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for (index, place) in self.typed.places.iter().enumerate() {
            let Some(reference) =
                crate::ty::reference_semantics_for_type(&place.ty, Some(&self.typed.type_registry))
            else {
                continue;
            };
            if reference.permission != crate::type_registry::ReferencePermission::Mutate {
                continue;
            }
            let root = PlaceId(index as u32);
            if !self.in_terminal_scope(root) {
                continue;
            }
            let incomplete = self.typed.places.iter().enumerate().any(|(child, info)| {
                if !matches!(
                    self.flow_ctx.place_state(PlaceId(child as u32)),
                    Some(VarState::Consumed | VarState::MaybeConsumed)
                ) || !self.typed.borrowed_value_needs_ownership(&info.ty)
                {
                    return false;
                }
                let mut parent = info.parent;
                while let Some(current) = parent {
                    let ancestor = &self.typed.places[current.0 as usize];
                    let ty = crate::ty::reference_pointee_for_type(
                        &ancestor.ty,
                        Some(&self.typed.type_registry),
                    )
                    .unwrap_or_else(|| ancestor.ty.clone());
                    if matches!(
                        self.typed.type_registry.resolved_definition_for_type(&ty),
                        Ty::Ptr { .. } | Ty::OwnedStorage { .. } | Ty::Address { .. }
                    ) {
                        return false;
                    }
                    if current == root {
                        return true;
                    }
                    parent = ancestor.parent;
                }
                false
            });
            if incomplete {
                diagnostics.push(
                    // type-move-out-of-borrow
                    Diagnostic::new(
                        OwnershipCode::MoveOutOfBorrow,
                        "borrowed storage must be initialized before the borrow ends",
                    )
                    .with_arg("name", place.name.as_str()),
                );
            }
        }

        diagnostics
    }

    pub(super) fn validate_terminal_exit(&mut self, site: Option<ExprId>) {
        if let Some(scope) = &mut self.local_scope {
            scope.exits.push(self.flow_ctx.clone());
        }
        let exit = if site.is_none()
            && self.definition.is_some_and(|definition| {
                self.typed
                    .span_table
                    .get(self.typed.defs[&definition].terminal_exit_span)
                    .is_empty()
            }) {
            TerminalExit::FunctionEnd
        } else {
            TerminalExit::Return
        };
        let mut diagnostics = self.pending_value_diagnostics(exit);
        diagnostics.extend(
            self.terminal_field_obligations()
                .into_iter()
                .map(|(name, ty)| self.terminal_field_diagnostic(name, &ty, exit)),
        );
        diagnostics.extend(self.incomplete_borrow_diagnostics());
        diagnostics.extend(self.unconsumed_owned_parameters());

        for diagnostic in diagnostics {
            if let Some(site) = site {
                let diagnostic = diagnostic.at_span_id(
                    self.typed.exprs.span[site.as_usize()],
                    &self.typed.span_table,
                );
                let flaws = &mut self.typed.exprs.facts.semantic_facts.flaws[site.as_usize()];
                if !flaws.iter().any(|flaw| flaw.message == diagnostic.message) {
                    flaws.push(diagnostic);
                }
            } else if let Some(definition) = self.definition {
                let bind = self.typed.defs.get_mut(&definition).unwrap();
                let diagnostic =
                    diagnostic.at_span_id(bind.terminal_exit_span, &self.typed.span_table);
                if !bind
                    .flaws
                    .iter()
                    .any(|flaw| flaw.message == diagnostic.message)
                {
                    bind.push_flaw(diagnostic);
                }
            }
        }
    }

    fn unconsumed_owned_parameters(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for place in &self.owned_parameters {
            let info = &self.typed.places[place.0 as usize];
            if self.flow_ctx.place_state(*place) == Some(VarState::Consumed)
                || info.ty.is_structurally_discardable()
            {
                continue;
            }
            let field_obligations = matches!(
                self.typed
                    .type_registry
                    .resolved_definition_for_type(&info.ty),
                Ty::Record { .. }
            ) && !self.typed.requires_terminal_authority(&info.ty);
            if field_obligations {
                continue;
            }
            for (code, kind) in [
                (
                    // type-owned-param-not-consumed
                    DiagnosticCode::Type(TypeCode::Ownership(OwnershipCode::ParameterNotConsumed)),
                    "parameter",
                ),
                (
                    // type-owned-value-not-consumed
                    DiagnosticCode::Type(TypeCode::Ownership(OwnershipCode::ValueNotConsumed)),
                    "value",
                ),
            ] {
                diagnostics.push(
                    Diagnostic::new(
                        code,
                        format!("owned {kind} `{}` is not returned or consumed", info.name),
                    )
                    .with_arg("name", info.name.as_str()),
                );
            }
        }
        diagnostics
    }

    pub(super) fn validate_unpack_authority(&mut self, value: ExprId, site: ExprId) {
        if let Some(ty) = self.typed.exprs.ty_of(value)
            && !self.typed.can_access_representation(ty)
        {
            // type-private-member
            let diagnostic = Diagnostic::new(
                MemberCode::Private,
                "cannot destructure a record containing private fields",
            )
            .at_span_id(
                self.typed.exprs.span[site.as_usize()],
                &self.typed.span_table,
            );
            self.typed.exprs.facts.semantic_facts.flaws[site.as_usize()].push(diagnostic);
            return;
        }
        if self.typed.borrows_value(value) {
            return;
        }
        let Some(ty) = self.typed.exprs.ty_of(value) else {
            return;
        };
        if self.typed.requires_terminal_authority(ty) && !self.typed.owns_terminal_authority(ty) {
            // type-terminal-authority
            let diagnostic = Diagnostic::new(
                TerminalCode::Authority,
                "only the defining module can destructure this owned type",
            )
            .at_span_id(
                self.typed.exprs.span[site.as_usize()],
                &self.typed.span_table,
            );
            self.typed.exprs.facts.semantic_facts.flaws[site.as_usize()].push(diagnostic);
        }
    }

    pub(super) fn validate_discarded_result(&mut self, value: ExprId) {
        if self.typed.exprs.place_of(value).flatten().is_some()
            || matches!(
                self.typed.exprs.kind_of(value),
                Some(TypedExprKind::Bind { .. } | TypedExprKind::Destructure { .. })
            )
        {
            return;
        }
        let Some(ty) = self.typed.exprs.ty_of(value) else {
            return;
        };
        if self.pending_terminal_obligation(ty, None) {
            // type-terminal-value-not-consumed
            let diagnostic = Diagnostic::new(
                TerminalCode::ValueNotConsumed,
                "returned ownership must be transferred or passed to a terminal operation",
            )
            .at_span_id(
                self.typed.exprs.span[value.as_usize()],
                &self.typed.span_table,
            );
            self.typed.exprs.facts.semantic_facts.flaws[value.as_usize()].push(diagnostic);
        }
    }
}
