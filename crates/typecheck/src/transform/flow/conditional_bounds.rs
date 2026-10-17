use super::*;

impl WalkExprCtx<'_> {
    pub(super) fn remember_conditional_record(&mut self, expression: ExprId, body: ExprId) {
        if crate::analysis::expression_returns(self.typed.resolved_program(), body) {
            return;
        }
        let ty = self.typed.exprs.ty[expression.index()].clone();
        let Ty::Record { fields, .. } = self.typed.type_registry.resolved_definition_for_type(&ty)
        else {
            return;
        };
        let Some(NormalExpr::Var(source)) = super::super::symbolic_values::normal_expr(
            self.typed,
            Some(self.callable_signatures),
            self.local_const_values,
            body,
        ) else {
            return;
        };
        let NormalExpr::Var(destination) =
            super::super::symbolic_values::call_result_symbol(expression)
        else {
            unreachable!()
        };
        let mut pending: Vec<_> = fields
            .iter()
            .enumerate()
            .map(|(field, (_, ty))| (vec![field], ty.as_ref().clone()))
            .collect();
        let mut aliases = crate::subst::DepSubst::new();
        while let Some((path, ty)) = pending.pop() {
            if path.len() > 16 || aliases.consts.len() > 128 {
                return;
            }
            if super::super::symbolic_values::scalar_type(self.typed, &ty) {
                let NormalExpr::Var(field) =
                    super::super::symbolic_values::field_path_symbol(source, &path)
                else {
                    unreachable!()
                };
                aliases.consts.insert(
                    field,
                    super::super::symbolic_values::field_path_symbol(destination, &path),
                );
            } else if let Ty::Record { fields, .. } =
                self.typed.type_registry.resolved_definition_for_type(&ty)
            {
                for (field, (_, ty)) in fields.iter().enumerate() {
                    let mut path = path.clone();
                    path.push(field);
                    pending.push((path, ty.as_ref().clone()));
                }
            }
        }
        for _ in 0..16 {
            let previous = aliases.consts.len();
            for (left, right) in &self.flow_ctx.constraint_env.known_eq {
                if let (NormalExpr::Var(left), NormalExpr::Var(right)) = (left, right) {
                    for (source, destination) in [(left, right), (right, left)] {
                        if let Some(value) = aliases.consts.get(source).cloned() {
                            aliases.consts.entry(*destination).or_insert(value);
                        }
                    }
                }
            }
            if aliases.consts.len() > 128 {
                return;
            }
            if aliases.consts.len() == previous {
                break;
            }
        }
        let constraints = &mut self.flow_ctx.constraint_env;
        for facts in [
            &mut constraints.known_eq,
            &mut constraints.known_lt,
            &mut constraints.known_le,
            &mut constraints.known_divisible,
        ] {
            let projected: Vec<_> = facts
                .iter()
                .map(|(left, right)| {
                    (
                        aliases.apply_to_normal(left),
                        aliases.apply_to_normal(right),
                    )
                })
                .collect();
            for fact in projected {
                if !facts.contains(&fact) {
                    facts.push(fact);
                }
            }
        }
    }

    pub(super) fn remember_conditional_bounds(
        &mut self,
        expression: ExprId,
        subject: Option<ExprId>,
        arms: &[TypedWhenArm],
    ) {
        if self.changes_record_facts(expression)
            || !matches!(arms.last(), Some(TypedWhenArm::Else(_, _)))
            || self
                .typed
                .type_registry
                .integer_validity_for_type(&self.typed.exprs.ty[expression.index()])
                .is_none()
        {
            return;
        }
        let mut conditions = Vec::new();
        let mut branches = Vec::new();
        let mut candidates = HashSet::new();
        for arm in arms {
            let (condition, body) = match arm {
                TypedWhenArm::Cond {
                    condition, body, ..
                } => (Some(condition.clone()), *body),
                TypedWhenArm::Is { pattern, body, .. } if subject.is_some() => (
                    Some(crate::typed::TypedCondition::Is {
                        subject: subject.unwrap(),
                        pattern: pattern.clone(),
                    }),
                    *body,
                ),
                TypedWhenArm::Else(body, _) => (None, *body),
                _ => return,
            };
            let Some(value) = super::super::symbolic_values::normal_expr(
                self.typed,
                Some(self.callable_signatures),
                self.local_const_values,
                body,
            ) else {
                return;
            };
            let mut constraints = self.flow_ctx.constraint_env.clone();
            for (condition, taken) in conditions
                .iter()
                .map(|condition| (condition, false))
                .chain(condition.as_ref().map(|condition| (condition, true)))
            {
                for (left, right, strict) in condition_comparison_facts(
                    condition,
                    taken,
                    self.typed,
                    Some(self.callable_signatures),
                    self.local_const_values,
                    Some(&self.flow_ctx.comparisons),
                    None,
                ) {
                    candidates.insert(left.clone());
                    candidates.insert(right.clone());
                    constraints.assume(&if strict {
                        Predicate::Lt(left, right)
                    } else {
                        Predicate::Le(left, right)
                    });
                }
            }
            if let Some(condition) = condition {
                conditions.push(condition);
            }
            candidates.insert(value.clone());
            if candidates.len() > 128 {
                return;
            }
            branches.push((value, constraints));
        }
        let result = super::super::symbolic_values::call_result_symbol(expression);
        let mut candidates: Vec<_> = candidates.into_iter().collect();
        candidates.sort_by_key(|candidate| format!("{candidate:?}"));
        let substitution = crate::subst::DepSubst::default();
        let branches: Vec<_> = branches
            .into_iter()
            .map(|(value, constraints)| (value, constraints.into_prepared_proofs(&substitution)))
            .collect();
        for candidate in candidates {
            if branches.iter().all(|(value, constraints)| {
                constraints.prove(&Predicate::Le(candidate.clone(), value.clone()))
                    == ProveResult::Proven
            }) {
                self.flow_ctx
                    .constraint_env
                    .assume(&Predicate::Le(candidate.clone(), result.clone()));
            }
            if branches.iter().all(|(value, constraints)| {
                constraints.prove(&Predicate::Le(value.clone(), candidate.clone()))
                    == ProveResult::Proven
            }) {
                self.flow_ctx
                    .constraint_env
                    .assume(&Predicate::Le(result.clone(), candidate));
            }
        }
    }
}
