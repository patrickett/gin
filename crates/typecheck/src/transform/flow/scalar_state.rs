use super::*;
use crate::subst::DepSubst;
use crate::transform::scalar_mutations::{instantiate, variables};
use crate::transform::symbolic_values::{field_path_symbol, field_symbol, normal_expr};

impl WalkExprCtx<'_> {
    pub(super) fn expose_raw_address(&mut self, expression: ExprId) {
        if let Some(targets) = self.record_targets_for(expression) {
            self.flow_ctx.raw_exposed.extend(&targets);
        } else {
            for targets in self.flow_ctx.record_targets.values() {
                self.flow_ctx.raw_exposed.extend(targets);
            }
        }
    }

    fn unexposed_storage(&self, symbol: Intern<String>) -> bool {
        let root = Intern::from_ref(
            symbol
                .as_str()
                .split(".$field")
                .next()
                .unwrap_or(symbol.as_str()),
        );
        let Some(ty) = self.flow_ctx.binding_types.get(&root) else {
            return false;
        };
        if self.borrowed_storage_has_raw_alias(ty) {
            return false;
        }
        let Some(targets) = self.flow_ctx.record_targets.get(&root) else {
            return false;
        };
        let mut projected = targets.clone();
        for field in symbol.as_str().split(".$field").skip(1) {
            let Ok(index) = field.parse::<usize>() else {
                return false;
            };
            projected = projected.projected(|base| ReferenceTargetGroup::Field {
                base: Box::new(base.clone()),
                index,
            });
        }
        projected.is_disjoint(&self.flow_ctx.raw_exposed, &self.flow_ctx.constraint_env)
    }

    pub(super) fn borrowed_storage_has_raw_alias(&self, ty: &Ty) -> bool {
        is_reference_type(self.typed, ty)
            && self
                .definition
                .and_then(|id| self.typed.defs.get(&id))
                .is_none_or(|bind| {
                    bind.params
                        .iter()
                        .any(|(_, ty)| self.contains_raw_alias(ty))
                })
    }

    fn contains_raw_alias(&self, ty: &Ty) -> bool {
        let mut pending = vec![ty.clone()];
        let mut visited = HashSet::new();
        for _ in 0..128 {
            let Some(ty) = pending.pop() else {
                return false;
            };
            if !visited.insert(ty.clone()) {
                continue;
            }
            match self.typed.type_registry.resolved_definition_for_type(&ty) {
                Ty::Ptr { .. } | Ty::Address { .. } => return true,
                Ty::Record { fields, .. } => {
                    pending.extend(fields.iter().map(|(_, ty)| ty.as_ref().clone()))
                }
                Ty::Array { elem, .. } | Ty::Ref { inner: elem, .. } => {
                    pending.push(elem.as_ref().clone())
                }
                Ty::Sequence(elements) => {
                    pending.extend(elements.iter().map(|element| element.ty.as_ref().clone()))
                }
                Ty::Union { variants, .. } => {
                    pending.extend(variants.iter().flat_map(|variant| {
                        variant.fields.iter().map(|(_, ty)| ty.as_ref().clone())
                    }))
                }
                _ => {}
            }
        }
        true
    }

    fn scalar_binding(&self, name: Intern<String>) -> bool {
        self.flow_ctx.scalar_bindings.contains(&name)
    }

    pub(super) fn capture_scalar_argument(
        &mut self,
        call: ExprId,
        slot: usize,
        argument: ExprId,
    ) -> Option<NormalExpr> {
        let ty = self.typed.exprs.ty_of(argument)?;
        if !is_reference_type(self.typed, ty)
            && matches!(
                self.typed.type_registry.resolved_definition_for_type(ty),
                Ty::Record { .. }
            )
        {
            let ty = ty.clone();
            let value = normal_expr(
                self.typed,
                Some(self.callable_signatures),
                self.local_const_values,
                argument,
            )?;
            self.next_field_snapshot += 1;
            let root = Intern::new(format!(
                "$argument{}.{}.{slot}",
                call.0, self.next_field_snapshot
            ));
            self.remember_record_value(root, &ty, &value);
            return Some(NormalExpr::Var(root));
        }
        if is_reference_type(self.typed, ty)
            || !crate::transform::symbolic_values::scalar_type(self.typed, ty)
        {
            return None;
        }
        let value = normal_expr(
            self.typed,
            Some(self.callable_signatures),
            self.local_const_values,
            argument,
        )
        .filter(|_| !self.changes_record_facts(argument));
        if let Some(value @ NormalExpr::Value(_)) = &value {
            return Some(value.clone());
        }
        self.next_field_snapshot += 1;
        let snapshot = NormalExpr::Var(Intern::new(format!(
            "$argument{}.{}.{slot}",
            call.0, self.next_field_snapshot
        )));
        if let Some(value) = value {
            self.flow_ctx
                .constraint_env
                .assume(&Predicate::Eq(snapshot.clone(), value));
        }
        Some(snapshot)
    }

    pub(super) fn remember_record_binding(
        &mut self,
        name: Intern<String>,
        body: ExprId,
        place: Option<PlaceId>,
    ) {
        self.flow_ctx.scalar_bindings.remove(&name);
        if let Some(ty) = place
            .and_then(|place| {
                self.typed
                    .places
                    .get(place.0 as usize)
                    .map(|place| &place.ty)
            })
            .or_else(|| self.typed.exprs.ty_of(body))
        {
            self.flow_ctx.binding_types.insert(name, ty.clone());
        }
        if self.typed.exprs.ty_of(body).is_some_and(|ty| {
            crate::transform::symbolic_values::scalar_type(
                self.typed,
                ty.without_reference_wrappers(),
            )
        }) {
            self.flow_ctx.scalar_bindings.insert(name);
        }
        let targets = if self
            .typed
            .exprs
            .ty_of(body)
            .is_some_and(|ty| is_reference_type(self.typed, ty))
        {
            self.record_targets_for(body)
        } else {
            place.map(|place| ReferenceTargetSet::singleton(ReferenceTargetGroup::Local(place)))
        };
        self.flow_ctx.record_targets.remove(&name);
        if let Some(targets) = targets.filter(|targets| targets.iter().next().is_some()) {
            self.flow_ctx.record_targets.insert(name, targets);
        }
    }

    pub(super) fn record_targets_for(&self, expression: ExprId) -> Option<ReferenceTargetSet> {
        if let Some(NormalExpr::Var(root)) = normal_expr(
            self.typed,
            Some(self.callable_signatures),
            &HashMap::new(),
            expression,
        ) && let Some(targets) = self.flow_ctx.record_targets.get(&root)
        {
            return Some(targets.clone());
        }
        if let Some(TypedExprKind::Ref(inner)) = self.typed.exprs.kind_of(expression)
            && matches!(
                self.typed.exprs.kind_of(*inner),
                Some(TypedExprKind::BufGet { .. })
            )
            && let Some(targets) = self
                .typed
                .exprs
                .target_group_of(expression)
                .cloned()
                .flatten()
        {
            return Some(targets);
        }
        if self
            .typed
            .exprs
            .ty_of(expression)
            .is_some_and(|ty| is_reference_type(self.typed, ty))
        {
            let references = self.typed.exprs.references_of(expression.index());
            if !references.sources.is_empty() {
                return self.reference_source_targets(references);
            }
        }
        self.typed
            .exprs
            .target_group_of(expression)
            .cloned()
            .flatten()
            .filter(|targets| targets.iter().next().is_some())
    }

    fn reference_source_targets(
        &self,
        references: &crate::typed::ReferenceValue,
    ) -> Option<ReferenceTargetSet> {
        if references
            .sources
            .iter()
            .all(|source| matches!(source, crate::typed::ReferenceSource::Target(_)))
        {
            return Some(references.targets());
        }
        let definition = self.definition.and_then(|id| self.typed.defs.get(&id))?;
        references.targets_in_definition(definition, &self.typed.type_registry)
    }

    pub(super) fn applied_effect_targets(
        &self,
        args: &[ExprId],
        signature: &TypedCallableSignature,
        effects: &HashSet<crate::typed::EffectTarget>,
    ) -> Vec<crate::typed::AppliedEffectTarget> {
        let mut applied =
            super::super::effects::applied_effect_targets_with(args, signature, effects, |arg| {
                self.record_targets_for(arg).map(std::borrow::Cow::Owned)
            });
        let mut resolved = Vec::new();
        for effect in applied.drain(..) {
            let targets = self
                .resolve_stored_reference_target(&effect.target, args)
                .unwrap_or_else(|| {
                    let mut targets = ReferenceTargetSet::default();
                    for known in self.flow_ctx.record_targets.values() {
                        targets.extend(known);
                    }
                    targets
                });
            for target in targets.iter() {
                let effect = crate::typed::AppliedEffectTarget {
                    target: target.clone(),
                    descendants: effect.descendants,
                };
                if !resolved.contains(&effect) {
                    resolved.push(effect);
                }
            }
        }
        resolved
    }

    fn reference_value_for_target<'a>(
        &'a self,
        target: &ReferenceTargetGroup,
        args: &[ExprId],
    ) -> Option<&'a crate::typed::ReferenceValue> {
        for arg in args {
            if self
                .record_targets_for(*arg)
                .is_some_and(|targets| targets.iter().any(|actual| actual == target))
            {
                return Some(self.typed.exprs.references_of(arg.index()));
            }
        }
        match target {
            ReferenceTargetGroup::Deref(base) => self.reference_value_for_target(base, args),
            ReferenceTargetGroup::Field { base, index } => self
                .reference_value_for_target(base, args)?
                .fields
                .get(*index),
            ReferenceTargetGroup::ItemRegion {
                base,
                index: crate::typed::TargetIndex::Symbolic(index),
            } => {
                let index = usize::try_from(index.as_const_int()?).ok()?;
                self.reference_value_for_target(base, args)?
                    .fields
                    .get(index)
            }
            _ => None,
        }
    }

    fn resolve_stored_reference_target(
        &self,
        target: &ReferenceTargetGroup,
        args: &[ExprId],
    ) -> Option<ReferenceTargetSet> {
        match target {
            ReferenceTargetGroup::Deref(base) => {
                let value = self.reference_value_for_target(base, args)?;
                if value.sources.is_empty() {
                    return None;
                }
                self.reference_source_targets(value)
            }
            ReferenceTargetGroup::Field { base, index } => Some(
                self.resolve_stored_reference_target(base, args)?
                    .projected(|base| ReferenceTargetGroup::Field {
                        base: Box::new(base.clone()),
                        index: *index,
                    }),
            ),
            ReferenceTargetGroup::Variant { base, name } => Some(
                self.resolve_stored_reference_target(base, args)?
                    .projected(|base| ReferenceTargetGroup::Variant {
                        base: Box::new(base.clone()),
                        name: *name,
                    }),
            ),
            ReferenceTargetGroup::ItemRegion { base, index } => Some(
                self.resolve_stored_reference_target(base, args)?
                    .projected(|base| ReferenceTargetGroup::ItemRegion {
                        base: Box::new(base.clone()),
                        index: index.clone(),
                    }),
            ),
            ReferenceTargetGroup::ItemRange { base, start, end } => Some(
                self.resolve_stored_reference_target(base, args)?
                    .projected(|base| ReferenceTargetGroup::ItemRange {
                        base: Box::new(base.clone()),
                        start: start.clone(),
                        end: end.clone(),
                    }),
            ),
            ReferenceTargetGroup::Param(_)
            | ReferenceTargetGroup::Local(_)
            | ReferenceTargetGroup::Static => Some(ReferenceTargetSet::singleton(target.clone())),
        }
    }

    pub(super) fn remember_record_alias(&mut self, name: Intern<String>, body: ExprId) {
        if !self
            .typed
            .exprs
            .ty_of(body)
            .is_some_and(|ty| is_reference_type(self.typed, ty))
        {
            return;
        }
        let Some(Ty::Record { fields, .. }) = self.typed.exprs.ty_of(body).map(|ty| {
            self.typed
                .type_registry
                .resolved_definition_for_type(ty.without_reference_wrappers())
        }) else {
            return;
        };
        let Some(targets) = self.flow_ctx.record_targets.get(&name) else {
            return;
        };
        if targets.iter().count() != 1 {
            return;
        }
        let aliases: Vec<_> = self
            .flow_ctx
            .record_targets
            .iter()
            .filter_map(|(alias, other)| (*alias != name && other == targets).then_some(*alias))
            .collect();
        for (field, (_, ty)) in fields.iter().enumerate() {
            if !crate::transform::symbolic_values::scalar_type(self.typed, ty) {
                continue;
            }
            for alias in &aliases {
                self.flow_ctx.constraint_env.assume(&Predicate::Eq(
                    field_symbol(name, field),
                    field_symbol(*alias, field),
                ));
            }
        }
    }

    fn state_symbols(&self, extra: &[NormalExpr]) -> HashSet<Intern<String>> {
        let mut names = HashSet::new();
        let constraints = &self.flow_ctx.constraint_env;
        for (left, right) in constraints
            .known_eq
            .iter()
            .chain(&constraints.known_lt)
            .chain(&constraints.known_le)
            .chain(&constraints.known_divisible)
        {
            variables(left, &mut names);
            variables(right, &mut names);
        }
        for value in extra {
            variables(value, &mut names);
        }
        for value in self.flow_ctx.scalar_values.values() {
            variables(value, &mut names);
        }
        for comparison in self
            .flow_ctx
            .comparisons
            .values()
            .chain(self.flow_ctx.binding_comparisons.values())
            .flatten()
        {
            variables(&comparison.left, &mut names);
            variables(&comparison.right, &mut names);
        }
        names
    }

    pub(super) fn snapshot_fields(
        &mut self,
        expression: ExprId,
        targets: Option<&ReferenceTargetSet>,
        extra: &[NormalExpr],
    ) -> DepSubst {
        self.forget_union_variant_exclusions(targets);
        let mut names = self.state_symbols(extra);
        for (name, root_targets) in &self.flow_ctx.record_targets {
            if self.scalar_binding(*name)
                && targets.is_none_or(|targets| {
                    !root_targets.is_disjoint(targets, &self.flow_ctx.constraint_env)
                })
            {
                names.insert(*name);
            }
        }
        let names: Vec<_> = names
            .into_iter()
            .filter(|name| {
                if name.as_str().starts_with("$argument") || name.as_str().starts_with("$entry.") {
                    return false;
                }
                if targets.is_none() && self.unexposed_storage(*name) {
                    return false;
                }
                if self
                    .definition
                    .and_then(|id| self.typed.defs.get(&id))
                    .is_some_and(|bind| {
                        super::super::postconditions::stable_value_input(
                            self.typed,
                            bind,
                            *name,
                            self.callable_signatures,
                        )
                    })
                {
                    return false;
                }

                let Some((root, path)) = name.as_str().split_once(".$field") else {
                    return self.scalar_binding(*name)
                        && self
                            .flow_ctx
                            .record_targets
                            .get(name)
                            .is_some_and(|root_targets| {
                                targets.is_none_or(|targets| {
                                    !root_targets
                                        .is_disjoint(targets, &self.flow_ctx.constraint_env)
                                })
                            });
                };
                targets.is_none_or(|targets| {
                    self.flow_ctx
                        .record_targets
                        .get(&Intern::from_ref(root))
                        .is_none_or(|root_targets| {
                            let mut fields = root_targets.clone();
                            for field in path.split(".$field") {
                                let Ok(index) = field.parse::<usize>() else {
                                    return true;
                                };
                                fields = fields.projected(|base| ReferenceTargetGroup::Field {
                                    base: Box::new(base.clone()),
                                    index,
                                });
                            }
                            !fields.is_disjoint(targets, &self.flow_ctx.constraint_env)
                        })
                })
            })
            .collect();
        let substitution = self.snapshot_symbols(expression, names);
        let constraints = &self.flow_ctx.constraint_env;
        self.local_const_values.retain(|name, value| {
            if substitution.consts.contains_key(name) {
                return false;
            }
            !matches!(value, ConstValue::Record { .. } | ConstValue::Tag { .. })
                || targets.is_some_and(|targets| {
                    self.flow_ctx
                        .record_targets
                        .get(name)
                        .is_some_and(|record| record.is_disjoint(targets, constraints))
                })
        });
        substitution
    }

    pub(super) fn is_local_reference_binding(&self, expression: ExprId) -> bool {
        self.typed
            .exprs
            .place_of(expression)
            .flatten()
            .and_then(|place| self.typed.places.get(place.0 as usize))
            .is_some_and(|place| {
                place.parent.is_none()
                    && place.projection.is_none()
                    && matches!(place.binder.owner, ast::BinderOwner::Expression(_))
                    && is_reference_type(self.typed, &place.ty)
            })
    }

    pub(super) fn snapshot_reference_binding(
        &mut self,
        expression: ExprId,
        name: Intern<String>,
    ) -> DepSubst {
        let targets = self.flow_ctx.record_targets.get(&name).cloned();
        self.forget_union_variant_exclusions(targets.as_ref());
        let names = self
            .state_symbols(&[])
            .into_iter()
            .filter(|symbol| {
                *symbol == name
                    || symbol
                        .as_str()
                        .split_once(".$field")
                        .is_some_and(|(root, _)| root == name.as_str())
            })
            .collect();
        let snapshot = self.snapshot_symbols(expression, names);
        self.local_const_values.remove(&name);
        snapshot
    }

    fn snapshot_symbols(&mut self, expression: ExprId, mut names: Vec<Intern<String>>) -> DepSubst {
        names.sort_by_key(|name| name.as_str().to_string());
        self.next_field_snapshot += 1;
        let version = self
            .typed
            .exprs
            .place_version_of(expression)
            .flatten()
            .map(|version| version.0)
            .unwrap_or(expression.0);
        let substitution = DepSubst {
            consts: names
                .into_iter()
                .enumerate()
                .map(|(slot, name)| {
                    (
                        name,
                        NormalExpr::Var(Intern::new(format!(
                            "$state{version}.{}.{}",
                            self.next_field_snapshot, slot
                        ))),
                    )
                })
                .collect(),
            ..Default::default()
        };
        let constraints = &mut self.flow_ctx.constraint_env;
        for (left, right) in constraints
            .known_eq
            .iter_mut()
            .chain(&mut constraints.known_lt)
            .chain(&mut constraints.known_le)
            .chain(&mut constraints.known_divisible)
        {
            *left = substitution.apply_to_normal(left);
            *right = substitution.apply_to_normal(right);
        }
        for value in self.flow_ctx.scalar_values.values_mut() {
            *value = substitution.apply_to_normal(value);
        }
        for comparison in self
            .flow_ctx
            .comparisons
            .values_mut()
            .chain(self.flow_ctx.binding_comparisons.values_mut())
            .flatten()
        {
            comparison.left = substitution.apply_to_normal(&comparison.left);
            comparison.right = substitution.apply_to_normal(&comparison.right);
        }
        self.const_bindings
            .retain(|name, _| !substitution.consts.contains_key(name));
        substitution
    }

    fn remember_scalar_field(&mut self, root: Intern<String>, field: usize, value: NormalExpr) {
        let aliases: Vec<_> = self
            .flow_ctx
            .record_targets
            .get(&root)
            .map(|targets| {
                self.flow_ctx
                    .record_targets
                    .iter()
                    .filter_map(|(alias, other)| {
                        (*alias == root || (targets.iter().count() == 1 && other == targets))
                            .then_some(*alias)
                    })
                    .collect()
            })
            .unwrap_or_else(|| vec![root]);
        for alias in aliases {
            self.flow_ctx
                .constraint_env
                .assume(&Predicate::Eq(field_symbol(alias, field), value.clone()));
        }
    }

    pub(super) fn apply_scalar_call_effects(
        &mut self,
        expression: ExprId,
        args: &[ExprId],
        argument_values: &[Option<NormalExpr>],
        signature: &TypedCallableSignature,
    ) {
        for exposed in self.applied_effect_targets(args, signature, &signature.effects.raw_exposes)
        {
            self.flow_ctx
                .raw_exposed
                .extend(&ReferenceTargetSet::singleton(exposed.target));
        }
        let mut targets = ReferenceTargetSet::default();
        let mutation = signature.effects.unknown_memory || !signature.effects.writes.is_empty();
        let mut arguments = HashMap::new();
        for (slot, ((name, _), arg)) in signature.params.iter().zip(args).enumerate() {
            if let Some(value) = argument_values.get(slot).cloned().flatten().or_else(|| {
                normal_expr(
                    self.typed,
                    Some(self.callable_signatures),
                    self.local_const_values,
                    *arg,
                )
            }) {
                arguments.insert(*name, value);
            }
            if signature.param_conventions.get(slot) == Some(&ParamConvention::Mutate) {
                if self.record_targets_for(*arg).is_none() {
                    let mut unknown_targets = ReferenceTargetSet::default();
                    for targets in self.flow_ctx.record_targets.values() {
                        unknown_targets.extend(targets);
                    }
                    self.snapshot_fields(expression, Some(&unknown_targets), &[]);
                    return;
                }
            }
        }
        if !mutation {
            return;
        }
        for applied in self.applied_effect_targets(args, signature, &signature.effects.writes) {
            targets.extend(&ReferenceTargetSet::singleton(applied.target));
        }
        let mut updates = Vec::new();
        {
            for effect in &signature.effects.scalar_fields {
                let Some(arg) = args.get(effect.parameter) else {
                    continue;
                };
                if let Some((parameter, _)) = signature.params.get(effect.parameter)
                    && effect.value == field_path_symbol(*parameter, &effect.field)
                    && let Some(actual) = self.record_targets_for(*arg)
                {
                    for (root, targets) in &self.flow_ctx.record_targets {
                        if targets.iter().count() == 1
                            && targets
                                .iter()
                                .all(|target| actual.iter().any(|actual| actual == target))
                        {
                            updates.push((
                                *root,
                                effect.field.clone(),
                                field_path_symbol(*root, &effect.field),
                            ));
                        }
                    }
                    continue;
                }
                let Some(NormalExpr::Var(root)) = normal_expr(
                    self.typed,
                    Some(self.callable_signatures),
                    &HashMap::new(),
                    *arg,
                ) else {
                    continue;
                };
                if let Some(value) = instantiate(&effect.value, &arguments) {
                    updates.push((root, effect.field.clone(), value));
                }
            }
        }
        if signature.effects.unknown_memory && targets.iter().next().is_some() {
            let snapshots = self.snapshot_fields(expression, Some(&targets), &[]);
            for (_, _, value) in &mut updates {
                *value = snapshots.apply_to_normal(value);
            }
        }
        let mut extra: Vec<_> = updates
            .iter()
            .map(|(root, field, _)| field_path_symbol(*root, field))
            .collect();
        extra.extend(updates.iter().map(|(_, _, value)| value.clone()));
        let targets = (!signature.effects.unknown_memory && targets.iter().next().is_some())
            .then_some(&targets);
        let snapshots = self.snapshot_fields(expression, targets, &extra);
        for (root, field, value) in updates {
            let (field, parents) = field.split_last().unwrap();
            let NormalExpr::Var(parent) = field_path_symbol(root, parents) else {
                unreachable!()
            };
            self.remember_scalar_field(parent, *field, snapshots.apply_to_normal(&value));
        }
    }

    pub(super) fn remember_field_write(&mut self, expression: ExprId, value: ExprId) {
        if let Some(TypedExprKind::BufSet { buf, index, .. }) = self.typed.exprs.kind_of(expression)
        {
            if super::super::effects::is_raw_pointer_buffer(self.typed, *buf) {
                self.snapshot_fields(expression, None, &[]);
                return;
            }
            let targets = self.record_targets_for(*buf).map(|targets| {
                targets.projected(|base| ReferenceTargetGroup::ItemRegion {
                    base: Box::new(base.clone()),
                    index: super::super::lower_exprs::target_index_for_expr(self.typed, *index),
                })
            });
            self.snapshot_fields(expression, targets.as_ref(), &[]);
            return;
        }
        let (base, field, has_operator) = match self.typed.exprs.kind_of(expression) {
            Some(TypedExprKind::AggregateSet {
                base,
                index,
                operator,
                ..
            }) => (*base, *index, operator.is_some()),
            Some(TypedExprKind::RecordSet {
                base,
                field,
                operator,
                ..
            }) => {
                let Some(index) = record_field_index(self.typed, *base, *field) else {
                    self.forget_record_facts();
                    return;
                };
                (*base, index, operator.is_some())
            }
            _ => {
                self.forget_record_facts();
                return;
            }
        };
        let Some(NormalExpr::Var(root)) = normal_expr(
            self.typed,
            Some(self.callable_signatures),
            &HashMap::new(),
            base,
        ) else {
            self.forget_record_facts();
            return;
        };
        let Some(Ty::Record { fields, .. }) = self.typed.exprs.ty_of(base).map(|ty| {
            self.typed
                .type_registry
                .resolved_definition_for_type(ty.without_reference_wrappers())
        }) else {
            self.forget_record_facts();
            return;
        };
        let fields = fields.clone();
        let replacement = (!has_operator)
            .then(|| {
                normal_expr(
                    self.typed,
                    Some(self.callable_signatures),
                    self.local_const_values,
                    value,
                )
            })
            .flatten();
        let record_replacement = fields.get(field).and_then(|(_, ty)| {
            if is_reference_type(self.typed, ty)
                || !matches!(
                    self.typed.type_registry.resolved_definition_for_type(ty),
                    Ty::Record { .. }
                )
            {
                return None;
            }
            let value = replacement.as_ref()?;
            self.next_field_snapshot += 1;
            let snapshot = Intern::new(format!(
                "$argumentwrite{}.{}",
                expression.0, self.next_field_snapshot
            ));
            self.remember_record_value(snapshot, ty, value);
            Some((ty.as_ref().clone(), snapshot))
        });
        let mut extra: Vec<_> = fields
            .iter()
            .enumerate()
            .map(|(index, _)| field_symbol(root, index))
            .collect();
        extra.extend(replacement.iter().cloned());
        let targets = self.record_targets_for(base).map(|targets| {
            targets.projected(|base| ReferenceTargetGroup::Field {
                base: Box::new(base.clone()),
                index: field,
            })
        });
        let snapshots = self.snapshot_fields(expression, targets.as_ref(), &extra);
        for (index, (_, ty)) in fields.iter().enumerate() {
            if !crate::transform::symbolic_values::scalar_type(self.typed, ty) {
                continue;
            }
            let value = if index == field {
                replacement.clone()
            } else {
                Some(field_symbol(root, index))
            };
            if let Some(value) = value {
                self.remember_scalar_field(root, index, snapshots.apply_to_normal(&value));
            }
        }
        if let Some((ty, snapshot)) = record_replacement
            && let NormalExpr::Var(destination) = field_symbol(root, field)
        {
            self.remember_record_value(destination, &ty, &NormalExpr::Var(snapshot));
        }
    }
}
