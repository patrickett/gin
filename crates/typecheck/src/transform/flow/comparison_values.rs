use super::*;
use crate::analysis::ScalarComparison;
use diagnostic::code::PropositionCode;

#[derive(Default)]
pub(super) struct ConditionValues {
    links: crate::subst::DepSubst,
    complete: [bool; 2],
}

pub(super) fn proposition_comparisons(
    ty: &Ty,
    values: &HashMap<Intern<String>, NormalExpr>,
) -> Option<Vec<ScalarComparison>> {
    let Ty::Proposition(predicate) = ty.without_reference_wrappers() else {
        return None;
    };
    let ast::ProofProposition::Compare {
        left,
        relation,
        right,
    } = predicate.as_ref()
    else {
        return None;
    };
    let left = crate::solver::proof_term_to_normal(left, &NormalExpr::from(0), values)?;
    let right = crate::solver::proof_term_to_normal(right, &NormalExpr::from(0), values)?;
    let comparison = match relation {
        ast::ProofRelation::Equal => crate::typed::MathematicalComparison::Equal,
        ast::ProofRelation::NotEqual => crate::typed::MathematicalComparison::NotEqual,
        ast::ProofRelation::Less => crate::typed::MathematicalComparison::Less,
        ast::ProofRelation::LessOrEqual => crate::typed::MathematicalComparison::LessOrEqual,
        ast::ProofRelation::Greater => crate::typed::MathematicalComparison::Greater,
        ast::ProofRelation::GreaterOrEqual => crate::typed::MathematicalComparison::GreaterOrEqual,
    };
    Some(vec![
        ScalarComparison {
            outcome: crate::analysis::ComparisonOutcome::Proposition(true),
            left: left.clone(),
            right: right.clone(),
            comparison,
        },
        ScalarComparison {
            outcome: crate::analysis::ComparisonOutcome::Proposition(false),
            left,
            right,
            comparison: comparison.complement(),
        },
    ])
}

impl WalkExprCtx<'_> {
    pub(super) fn check_proposition_arguments(
        &mut self,
        call: ExprId,
        parameters: &[(Intern<String>, Ty)],
        arguments: &[ExprId],
        argument_values: &[Option<NormalExpr>],
    ) {
        let values = parameters
            .iter()
            .zip(arguments)
            .enumerate()
            .filter_map(|(slot, ((name, _), argument))| {
                argument_values
                    .get(slot)
                    .cloned()
                    .flatten()
                    .or_else(|| self.flow_ctx.scalar_values.get(argument).cloned())
                    .map(|value| (*name, value))
            })
            .collect();
        for ((_, ty), argument) in parameters.iter().zip(arguments) {
            if !matches!(ty.without_reference_wrappers(), Ty::Proposition(_)) {
                continue;
            }
            let required = proposition_comparisons(ty, &values);
            let actual = self.flow_ctx.comparisons.get(argument);
            let matches = required
                .as_ref()
                .zip(actual)
                .is_some_and(|(required, actual)| {
                    required.iter().all(|required| {
                        actual.iter().any(|actual| {
                            required.outcome == actual.outcome
                                && required.comparison == actual.comparison
                                && [&required.left, &required.right]
                                    .into_iter()
                                    .zip([&actual.left, &actual.right])
                                    .all(|(required, actual)| {
                                        self.flow_ctx.constraint_env.prove(
                                            &Predicate::Eq(required.clone(), actual.clone()),
                                            &HashMap::new(),
                                        ) == ProveResult::Proven
                                    })
                        })
                    })
                });
            if !matches {
                // type-proposition-argument-unproven
                self.typed.exprs.facts.semantic_facts.flaws[call.index()].push(Diagnostic::new(
                    PropositionCode::ArgumentUnproven, "cannot establish that this proposition tested the supplied values; check them again before this call",
                ));
            }
        }
    }
    fn all_condition_effects_run(
        &self,
        condition: &crate::typed::TypedCondition,
        taken: bool,
    ) -> bool {
        use crate::typed::TypedCondition as C;
        let pure = |condition: &C| {
            let mut changes = false;
            condition.visit_subjects(&mut |subject| changes |= self.changes_record_facts(subject));
            !changes
        };
        match condition {
            C::Proposition { .. } | C::Is { .. } | C::Compare { .. } => true,
            C::Not(inner) => self.all_condition_effects_run(inner, !taken),
            C::And(left, right) if taken => {
                self.all_condition_effects_run(left, true)
                    && self.all_condition_effects_run(right, true)
            }
            C::Or(left, right) if !taken => {
                self.all_condition_effects_run(left, false)
                    && self.all_condition_effects_run(right, false)
            }
            C::And(left, right) | C::Or(left, right) => {
                pure(right)
                    && self.all_condition_effects_run(left, true)
                    && self.all_condition_effects_run(left, false)
            }
        }
    }

    pub(super) fn walk_condition_values(
        &mut self,
        condition: &crate::typed::TypedCondition,
    ) -> ConditionValues {
        fn conditional(condition: &crate::typed::TypedCondition) -> bool {
            match condition {
                crate::typed::TypedCondition::And(_, _)
                | crate::typed::TypedCondition::Or(_, _) => true,
                crate::typed::TypedCondition::Not(inner) => conditional(inner),
                _ => false,
            }
        }
        let mut subjects = Vec::new();
        condition.visit_subjects(&mut |subject| subjects.push(subject));
        for subject in &subjects {
            self.walk(*subject);
        }
        if conditional(condition)
            && subjects
                .iter()
                .any(|subject| self.changes_record_facts(*subject))
            && let Some(site) = subjects.first()
        {
            ConditionValues {
                complete: [
                    self.all_condition_effects_run(condition, false),
                    self.all_condition_effects_run(condition, true),
                ],
                links: self.abstract_loop_state(*site, &subjects),
            }
        } else {
            ConditionValues::default()
        }
    }

    pub(super) fn condition_facts(
        &self,
        condition: &crate::typed::TypedCondition,
        taken: bool,
        values: &ConditionValues,
    ) -> Vec<(NormalExpr, NormalExpr, bool)> {
        let mut facts = condition_comparison_facts(
            condition,
            taken,
            self.typed,
            Some(self.callable_signatures),
            self.local_const_values,
            Some(&self.flow_ctx.comparisons),
            Some(&self.flow_ctx.scalar_values),
        );
        if values.complete[usize::from(taken)] {
            for (name, snapshot) in &values.links.consts {
                let current = NormalExpr::Var(*name);
                facts.push((current.clone(), snapshot.clone(), false));
                facts.push((snapshot.clone(), current, false));
            }
        }
        facts
    }

    fn snapshot_comparison(&self, expression: ExprId) -> Option<Vec<ScalarComparison>> {
        if let Some(TypedExprKind::FnCall {
            target,
            args: Some(_),
            ..
        }) = self.typed.exprs.kind_of(expression)
            && matches!(self.typed.exprs.ty_of(expression), Some(Ty::Proposition(_)))
        {
            let signature = self.callable_signatures.get(target)?;
            if signature.param_conventions.iter().any(|convention| {
                matches!(convention, ParamConvention::Own | ParamConvention::Mutate)
            }) || signature.effects.unknown_memory
                || !signature.effects.writes.is_empty()
            {
                return None;
            }
        }
        if let Some(crate::typed::EvidenceProposition::IntegerComparison {
            lhs,
            rhs,
            comparison,
        }) = self.typed.exprs.proposition_evidence_of(expression)
        {
            let operand = |operand: &ExprId| {
                self.flow_ctx
                    .scalar_values
                    .get(operand)
                    .cloned()
                    .or_else(|| {
                        self.typed
                            .exprs
                            .const_value_of(*operand)
                            .and_then(Option::as_ref)
                            .filter(|value| matches!(value, ast::ConstValue::Int(_)))
                            .cloned()
                            .map(NormalExpr::Value)
                    })
            };
            let left = operand(lhs)?;
            let right = operand(rhs)?;
            return Some(vec![
                ScalarComparison {
                    outcome: crate::analysis::ComparisonOutcome::Proposition(true),
                    left: left.clone(),
                    right: right.clone(),
                    comparison: *comparison,
                },
                ScalarComparison {
                    outcome: crate::analysis::ComparisonOutcome::Proposition(false),
                    left,
                    right,
                    comparison: comparison.complement(),
                },
            ]);
        }
        self.typed
            .exprs
            .result_evidence_of(expression)
            .and_then(|evidence| {
                evidence
                    .alternatives
                    .iter()
                    .map(|alternative| {
                        let crate::typed::EvidenceProposition::IntegerComparison {
                            lhs,
                            rhs,
                            comparison,
                        } = alternative.proposition;
                        Some(ScalarComparison {
                            outcome: crate::analysis::ComparisonOutcome::Alternative(
                                alternative.label,
                            ),
                            left: self.flow_ctx.scalar_values.get(&lhs)?.clone(),
                            right: self.flow_ctx.scalar_values.get(&rhs)?.clone(),
                            comparison,
                        })
                    })
                    .collect::<Option<Vec<_>>>()
            })
    }

    pub(super) fn remember_expression_values(&mut self, expression: ExprId) {
        let Some(kind) = self.typed.exprs.kind_of(expression) else {
            return;
        };
        if let Some(ty) = self.typed.exprs.ty_of(expression)
            && self
                .typed
                .type_registry
                .integer_validity_for_type(ty.without_reference_wrappers())
                .is_some()
        {
            self.flow_ctx.scalar_values.remove(&expression);
            if let Some(value) = super::super::symbolic_values::normal_expr(
                self.typed,
                Some(self.callable_signatures),
                self.local_const_values,
                expression,
            )
            .filter(|_| !self.changes_record_facts(expression))
            {
                self.flow_ctx.scalar_values.insert(expression, value);
            }
        }
        let comparisons = match kind {
            TypedExprKind::Bind { name, body, .. }
            | TypedExprKind::Reassign {
                name, value: body, ..
            } => {
                let value = self.flow_ctx.comparisons.get(body).cloned();
                if let Some(value) = &value {
                    self.flow_ctx
                        .binding_comparisons
                        .insert(*name, value.clone());
                } else {
                    self.flow_ctx.binding_comparisons.remove(name);
                }
                value
            }
            TypedExprKind::FnCall {
                target, args: None, ..
            } if self.typed.exprs.place_of(expression).flatten().is_some() => {
                self.flow_ctx.binding_comparisons.get(&target.0).cloned()
            }
            TypedExprKind::Ref(inner)
            | TypedExprKind::Deref(inner)
            | TypedExprKind::Cast { expr: inner, .. }
            | TypedExprKind::Rematerialize { source: inner, .. } => {
                self.flow_ctx.comparisons.get(inner).cloned()
            }
            TypedExprKind::FnCall {
                target,
                args: Some(args),
                ..
            } => {
                let forwarded = self.typed.defs.get(target).and_then(|bind| {
                    let returned = match bind.body {
                        crate::typed::BindBody::Expr(expression)
                        | crate::typed::BindBody::Body {
                            ret: Some(expression),
                            ..
                        } => expression,
                        _ => return None,
                    };
                    let TypedExprKind::FnCall {
                        target: parameter,
                        args: None,
                        ..
                    } = self.typed.exprs.kind_of(returned)?
                    else {
                        return None;
                    };
                    let slot = bind
                        .params
                        .iter()
                        .position(|(name, _)| *name == parameter.0)?;
                    args.get(slot).copied()
                });
                if let Some(argument) = forwarded {
                    self.flow_ctx.comparisons.get(&argument).cloned()
                } else {
                    self.snapshot_comparison(expression)
                }
            }
            TypedExprKind::Binary { .. } | TypedExprKind::IntrinsicCall { .. } => {
                self.snapshot_comparison(expression)
            }
            _ => None,
        };
        self.flow_ctx.comparisons.remove(&expression);
        if let Some(comparisons) = comparisons {
            self.flow_ctx.comparisons.insert(expression, comparisons);
        }
    }
}
