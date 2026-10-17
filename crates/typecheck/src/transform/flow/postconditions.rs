use super::*;
use crate::typed::TypedPostcondition;
use diagnostic::code::ProofCode;

pub(super) fn resolve(
    typed: &TypedFileAst,
    signatures: &CallableSignatureScope<'_>,
    types: &HashMap<Intern<String>, Ty>,
    mut values: HashMap<Intern<String>, NormalExpr>,
    proposition: &ast::ProofProposition,
) -> Option<Predicate> {
    let surface = ast::PredicateExpr::Proposition(Box::new(proposition.clone()));
    super::super::refinement_terms::extend_values(typed, signatures, types, &mut values, &surface);
    if !super::super::refinement_terms::values_complete(&surface, &values) {
        return None;
    }
    let predicate = predicate_expr_to_predicate(&surface, 0.into(), &values);
    Some(resolve_target_queries_in_predicate(
        &predicate,
        &crate::subst::DepSubst::new(),
        typed,
    ))
}

impl WalkExprCtx<'_> {
    pub(super) fn verify_postcondition(
        &mut self,
        proof: &TypedPostcondition,
        site: Option<ExprId>,
    ) {
        let outcome = super::super::refinement_types::validate_proposition(
            self.typed,
            self.callable_signatures,
            &proof.types,
            &proof.proposition,
        );
        let flaw = match outcome {
            Err(flaw) => Some(flaw),
            Ok(()) => {
                let values = self
                    .local_const_values
                    .iter()
                    .filter(|(_, value)| matches!(value, ConstValue::Int(_)))
                    .map(|(name, value)| (*name, NormalExpr::Value(value.clone())))
                    .collect();
                let result = resolve(
                    self.typed,
                    self.callable_signatures,
                    &proof.types,
                    values,
                    &proof.proposition,
                )
                .map(|predicate| {
                    self.flow_ctx
                        .constraint_env
                        .prove(&predicate, &HashMap::new())
                })
                .unwrap_or(ProveResult::Unknown);
                match result {
                    ProveResult::Proven => None,
                    // type-postcondition-failed
                    ProveResult::Disproven => Some(Diagnostic::new(
                        ProofCode::PostconditionFailed,
                        format!(
                            "return contradicts its `thus` postcondition `{}`",
                            proof.proposition
                        ),
                    )),
                    ProveResult::Unknown => {
                        Some(refinement_diagnostics::postcondition(&proof.proposition))
                    }
                }
            }
        };
        if let Some(flaw) = flaw {
            let flaw = flaw.at_span_id(proof.span, &self.typed.span_table);
            if let Some(site) = site {
                let flaws = &mut self.typed.exprs.facts.semantic_facts.flaws[site.index()];
                if !flaws.contains(&flaw) {
                    flaws.push(flaw);
                }
            } else if let Some(bind) = self.definition.and_then(|id| self.typed.defs.get_mut(&id)) {
                bind.push_flaw(flaw);
            }
        }
    }

    pub(super) fn apply_postconditions(
        &mut self,
        expression: ExprId,
        args: &[ExprId],
        captured: &[Option<NormalExpr>],
        signature: &TypedCallableSignature,
    ) {
        let mut substitutions = crate::subst::DepSubst::new();
        for ((_, formal), arg) in signature.params.iter().zip(args) {
            let actual = &self.typed.exprs.ty[arg.index()];
            collect_dependent_type_substitutions(
                formal.without_reference_wrappers(),
                actual.without_reference_wrappers(),
                &mut substitutions.types,
            );
        }
        let mut values = HashMap::new();
        let mut types = HashMap::new();
        let result = Intern::from_ref("$result");
        types.insert(result, self.typed.exprs.ty[expression.index()].clone());
        values.insert(
            result,
            super::super::symbolic_values::call_result_symbol(expression),
        );
        for (slot, ((name, ty), arg)) in signature.params.iter().zip(args).enumerate() {
            types.insert(*name, substitutions.apply_to_ty(ty));
            let value = self.postcondition_argument_value(
                *arg,
                captured.get(slot).cloned().flatten(),
                signature.param_conventions.get(slot).copied(),
            );
            if let Some(value) = value {
                values.insert(*name, value);
            }
        }
        self.apply_payload_postconditions(expression, signature, &substitutions, &types, &values);
        for proposition in &signature.effects.postconditions {
            let predicate = ast::PredicateExpr::Proposition(Box::new(proposition.clone()));
            if super::super::refinement_terms::predicate_names(&predicate)
                .iter()
                .any(|name| {
                    let root =
                        Intern::from_ref(name.as_str().split('.').next().unwrap_or(name.as_str()));
                    types.contains_key(&root) && !values.contains_key(&root)
                })
            {
                continue;
            }
            if let Some(predicate) = resolve(
                self.typed,
                self.callable_signatures,
                &types,
                values.clone(),
                proposition,
            ) {
                self.flow_ctx.constraint_env.assume(&predicate);
            }
        }
    }

    fn postcondition_argument_value(
        &self,
        arg: ExprId,
        captured: Option<NormalExpr>,
        convention: Option<ParamConvention>,
    ) -> Option<NormalExpr> {
        let current = || {
            super::super::symbolic_values::normal_expr(
                self.typed,
                Some(self.callable_signatures),
                self.local_const_values,
                arg,
            )
        };
        if convention == Some(ParamConvention::Mutate) {
            let Some(NormalExpr::Var(symbol)) = current() else {
                return None;
            };
            let mut path = symbol.as_str().split(".$field");
            let root = Intern::from_ref(path.next().unwrap_or(symbol.as_str()));
            let Some(mut expected) = self.flow_ctx.record_targets.get(&root).cloned() else {
                return None;
            };
            let mut complete = true;
            for field in path {
                let Ok(index) = field.parse::<usize>() else {
                    complete = false;
                    break;
                };
                expected = expected.projected(|base| ReferenceTargetGroup::Field {
                    base: Box::new(base.clone()),
                    index,
                });
            }
            if !complete || self.record_targets_for(arg).as_ref() != Some(&expected) {
                return None;
            }
            Some(NormalExpr::Var(symbol))
        } else {
            captured.or_else(current)
        }
    }
}
