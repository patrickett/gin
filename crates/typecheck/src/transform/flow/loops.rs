use super::*;

impl WalkExprCtx<'_> {
    fn loop_writes(
        &self,
        expressions: &[ExprId],
    ) -> Option<(ReferenceTargetSet, HashSet<Intern<String>>)> {
        let aliases = super::loop_targets::LoopTargets::new(self, expressions)?;
        let mut targets = ReferenceTargetSet::default();
        for expression in &aliases.expressions {
            targets.extend(&self.loop_expression_writes(&aliases, *expression)?);
        }
        Some((targets, aliases.rebound))
    }

    fn loop_expression_writes(
        &self,
        aliases: &super::loop_targets::LoopTargets,
        expression: ExprId,
    ) -> Option<ReferenceTargetSet> {
        let mut targets = ReferenceTargetSet::default();
        match self.typed.exprs.kind_of(expression)? {
            TypedExprKind::Emit {
                state: crate::assembly::AssemblyState::Checked(assembly),
                operands,
                ..
            } => {
                targets.extend(&self.loop_assembly_writes(aliases, assembly, operands)?);
            }
            TypedExprKind::Emit { .. } => return None,
            TypedExprKind::FnCall { target, args, .. }
                if args.is_some() || self.typed.exprs.place_of(expression).flatten().is_none() =>
            {
                let signature = self.callable_signatures.get(target)?;
                targets.extend(&self.loop_call_writes(
                    aliases,
                    args.as_deref().unwrap_or(&[]),
                    signature,
                )?);
            }
            TypedExprKind::Reassign { .. } if self.is_local_reference_binding(expression) => {}
            TypedExprKind::Reassign { name, .. } => {
                targets.extend(self.flow_ctx.record_targets.get(name)?)
            }
            TypedExprKind::AggregateSet { base, index, .. } => {
                targets.extend(&project_field(
                    &aliases.targets_for(self, *base)?,
                    &[*index],
                ));
            }
            TypedExprKind::RecordSet { base, field, .. } => {
                let index = record_field_index(self.typed, *base, *field)?;
                targets.extend(&project_field(&aliases.targets_for(self, *base)?, &[index]));
            }
            TypedExprKind::BufSet { buf, .. } => {
                if super::super::effects::is_raw_pointer_buffer(self.typed, *buf) {
                    targets.extend(&self.loop_raw_write_targets());
                } else {
                    targets.extend(&aliases.targets_for(self, *buf)?);
                }
            }
            TypedExprKind::IntrinsicCall {
                op: crate::intrinsic::IntrinsicOp::AddressStore,
                ..
            } => targets.extend(&self.loop_raw_write_targets()),
            _ => {}
        }
        Some(targets)
    }

    fn loop_assembly_writes(
        &self,
        aliases: &super::loop_targets::LoopTargets,
        assembly: &crate::assembly::CheckedAssembly,
        operands: &[(Intern<String>, ExprId)],
    ) -> Option<ReferenceTargetSet> {
        let mut targets = ReferenceTargetSet::default();
        if !assembly.memory.writes.is_empty() {
            targets.extend(&self.loop_raw_write_targets());
        }
        for (pointer, _) in &assembly.memory.writes {
            let (_, operand) = operands.iter().find(|(name, _)| name.as_str() == pointer)?;
            if self.typed.borrows_value(*operand) {
                targets.extend(&aliases.targets_for(self, *operand)?);
            }
        }
        Some(targets)
    }

    fn loop_call_writes(
        &self,
        aliases: &super::loop_targets::LoopTargets,
        args: &[ExprId],
        signature: &TypedCallableSignature,
    ) -> Option<ReferenceTargetSet> {
        let mut targets = ReferenceTargetSet::default();
        if signature.effects.unknown_memory {
            targets.extend(&self.loop_raw_write_targets());
        }
        targets.extend(&self.loop_call_targets(
            aliases,
            args,
            signature,
            &signature.effects.writes,
        )?);
        Some(targets)
    }

    fn loop_call_targets(
        &self,
        aliases: &super::loop_targets::LoopTargets,
        args: &[ExprId],
        signature: &TypedCallableSignature,
        effects: &HashSet<crate::typed::EffectTarget>,
    ) -> Option<ReferenceTargetSet> {
        let mut targets = ReferenceTargetSet::default();
        for effect in effects {
            let mut complete = false;
            for (slot, group) in signature.param_groups.iter().enumerate() {
                let Some(group) = group else {
                    continue;
                };
                for actual in aliases.targets_for(self, *args.get(slot)?)?.iter() {
                    if let Some(applied) = effect.substitute(*group, actual) {
                        targets.extend(&ReferenceTargetSet::singleton(applied.target));
                        complete = true;
                    }
                }
            }
            if !complete {
                return None;
            }
        }
        Some(targets)
    }

    fn loop_raw_write_targets(&self) -> ReferenceTargetSet {
        let mut targets = self.flow_ctx.raw_exposed.clone();
        for (name, ty) in &self.flow_ctx.binding_types {
            if self.borrowed_storage_has_raw_alias(ty) {
                if let Some(actual) = self.flow_ctx.record_targets.get(name) {
                    targets.extend(actual);
                }
            }
        }
        targets
    }

    fn loop_preserved_fields(&self, expressions: &[ExprId]) -> HashSet<Intern<String>> {
        let Some(aliases) = super::loop_targets::LoopTargets::new(self, expressions) else {
            return HashSet::new();
        };
        let mut names = HashSet::new();
        let constraints = &self.flow_ctx.constraint_env;
        for (left, right) in constraints
            .known_eq
            .iter()
            .chain(&constraints.known_le)
            .chain(&constraints.known_lt)
        {
            super::super::scalar_mutations::variables(left, &mut names);
            super::super::scalar_mutations::variables(right, &mut names);
        }
        names.retain(|name| {
            let mut parts = name.as_str().split(".$field");
            let root = Intern::from_ref(parts.next().unwrap_or_default());
            if aliases.rebound.contains(&root) {
                return false;
            }
            let Some(targets) = self.flow_ctx.record_targets.get(&root) else {
                return false;
            };
            let Some(path) = parts
                .map(|field| field.parse::<usize>().ok())
                .collect::<Option<Vec<_>>>()
            else {
                return false;
            };
            if path.is_empty() || path.len() > 16 {
                return false;
            }
            let field = project_field(targets, &path);
            aliases.expressions.iter().all(|expression| {
                let Some(writes) = self.loop_expression_writes(&aliases, *expression) else {
                    return false;
                };
                writes.is_disjoint(&field, constraints)
                    || self.loop_call_preserves(&aliases, *expression, &field)
            })
        });
        names
    }

    fn loop_call_preserves(
        &self,
        aliases: &super::loop_targets::LoopTargets,
        expression: ExprId,
        field: &ReferenceTargetSet,
    ) -> bool {
        let Some(TypedExprKind::FnCall {
            target,
            args: Some(args),
            ..
        }) = self.typed.exprs.kind_of(expression)
        else {
            return false;
        };
        let Some(signature) = self.callable_signatures.get(target) else {
            return false;
        };
        signature.effects.scalar_fields.iter().any(|effect| {
            let Some((name, _)) = signature.params.get(effect.parameter) else {
                return false;
            };
            if effect.value
                != super::super::symbolic_values::field_path_symbol(*name, &effect.field)
            {
                return false;
            }
            args.get(effect.parameter)
                .and_then(|arg| aliases.targets_for(self, *arg))
                .is_some_and(|targets| project_field(&targets, &effect.field) == *field)
        })
    }

    fn expose_loop_raw_addresses(&mut self, expressions: &[ExprId]) {
        let aliases = super::loop_targets::LoopTargets::new(self, expressions);
        for root in expressions {
            let mut exposures = Vec::new();
            let mut calls = Vec::new();
            let _ = crate::typed::walk_expr_preorder(self.typed, *root, &mut |expression| {
                match self.typed.exprs.kind_of(expression) {
                    Some(TypedExprKind::TakePtr(inner)) => exposures.push(*inner),
                    Some(TypedExprKind::Cast { expr, .. })
                        if super::super::effects::is_raw_pointer_buffer(self.typed, expression)
                            && self.typed.borrows_value(*expr) =>
                    {
                        exposures.push(*expr)
                    }
                    Some(TypedExprKind::FnCall {
                        target,
                        args: Some(args),
                        ..
                    }) => {
                        if let Some(signature) = self.callable_signatures.get(target) {
                            calls.push((args.clone(), signature.clone()));
                        }
                    }
                    _ => {}
                }
                std::ops::ControlFlow::Continue(())
            });
            for (args, signature) in calls {
                if !signature.effects.raw_exposes.is_empty() {
                    let exposed = aliases.as_ref().and_then(|aliases| {
                        self.loop_call_targets(
                            aliases,
                            &args,
                            &signature,
                            &signature.effects.raw_exposes,
                        )
                    });
                    self.expose_loop_targets(exposed);
                }
                for applied in
                    self.applied_effect_targets(&args, &signature, &signature.effects.raw_exposes)
                {
                    self.flow_ctx
                        .raw_exposed
                        .extend(&ReferenceTargetSet::singleton(applied.target));
                }
            }
            for exposure in exposures {
                let targets = aliases
                    .as_ref()
                    .and_then(|aliases| aliases.targets_for(self, exposure));
                self.expose_loop_targets(targets);
            }
        }
    }

    fn expose_loop_targets(&mut self, targets: Option<ReferenceTargetSet>) {
        if let Some(targets) = targets {
            self.flow_ctx.raw_exposed.extend(&targets);
        } else {
            for targets in self.flow_ctx.record_targets.values() {
                self.flow_ctx.raw_exposed.extend(targets);
            }
        }
    }

    pub(super) fn abstract_loop_state(
        &mut self,
        site: ExprId,
        expressions: &[ExprId],
    ) -> crate::subst::DepSubst {
        self.expose_loop_raw_addresses(expressions);
        let preserved = self.loop_preserved_fields(expressions);
        let analysis = self.loop_writes(expressions);
        let mut snapshot = crate::subst::DepSubst::new();
        if let Some((_, rebound)) = &analysis {
            let mut rebound: Vec<_> = rebound.iter().copied().collect();
            rebound.sort_by_key(|name| name.as_str().to_owned());
            for name in rebound {
                snapshot
                    .consts
                    .extend(self.snapshot_reference_binding(site, name).consts);
            }
        }
        let mut unknown_targets = ReferenceTargetSet::default();
        if analysis.is_none() {
            for targets in self.flow_ctx.record_targets.values() {
                unknown_targets.extend(targets);
            }
        }
        let targets = analysis
            .as_ref()
            .map_or(&unknown_targets, |(targets, _)| targets);
        snapshot
            .consts
            .extend(self.snapshot_fields(site, Some(targets), &[]).consts);
        for name in preserved {
            if let Some(value) = snapshot.consts.get(&name) {
                self.flow_ctx
                    .constraint_env
                    .assume(&Predicate::Eq(NormalExpr::Var(name), value.clone()));
            }
        }
        for (name, ty) in &self.flow_ctx.binding_types {
            record_constraints::assume_integer_bounds(
                self.typed,
                ty,
                NormalExpr::Var(*name),
                &mut self.flow_ctx.constraint_env,
            );
            record_constraints::assume_record_constraints(
                self.typed,
                self.callable_signatures,
                *name,
                ty,
                &mut self.flow_ctx.constraint_env,
            );
        }
        snapshot
    }
}

fn project_field(targets: &ReferenceTargetSet, path: &[usize]) -> ReferenceTargetSet {
    let mut targets = targets.clone();
    for index in path {
        targets = targets.projected(|base| ReferenceTargetGroup::Field {
            base: Box::new(base.clone()),
            index: *index,
        });
    }
    targets
}
