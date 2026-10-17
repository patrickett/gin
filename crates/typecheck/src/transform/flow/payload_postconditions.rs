use super::*;
use crate::transform::symbolic_values::{normal_expr, payload_symbol};
use diagnostic::code::{ProofCode, ResultCode};

pub(super) fn result_exits(typed: &TypedFileAst, body: &crate::typed::BindBody) -> HashSet<ExprId> {
    let mut pending = crate::analysis::return_expressions(typed, body);
    let mut visited = HashSet::new();
    let mut exits = HashSet::new();
    while let Some(expression) = pending.pop() {
        if !visited.insert(expression) {
            continue;
        }
        let nested =
            crate::analysis::return_expressions(typed, &crate::typed::BindBody::Expr(expression));
        if nested.as_slice() == [expression] {
            exits.insert(expression);
        } else {
            pending.extend(nested);
        }
    }
    exits
}

pub(super) fn validate_contracts(
    typed: &mut TypedFileAst,
    signatures: &CallableSignatureScope<'_>,
) {
    let definitions: Vec<_> = typed
        .defs
        .iter()
        .filter(|(_, bind)| !bind.payload_postconditions.is_empty())
        .map(|(id, bind)| (*id, bind.clone()))
        .collect();
    for (id, bind) in definitions {
        for proof in bind.payload_postconditions.iter() {
            let mut types: HashMap<_, _> = bind.params.iter().cloned().collect();
            let flaw = if types.contains_key(&proof.name) {
                // type-result-name-conflict
                Some(Diagnostic::new(
                    ResultCode::NameConflict,
                    format!("result payload `{}` shadows a parameter", proof.name),
                ))
            } else if bind.return_type != proof.ty
                && payload_index(typed, &bind.return_type, &proof.ty).is_none()
            {
                // type-result-payload-mismatch
                Some(Diagnostic::new(
                    ResultCode::PayloadMismatch,
                    "a payload contract must describe a returned type",
                ))
            } else {
                types.insert(proof.name, proof.ty.clone());
                super::super::refinement_types::validate_proposition(
                    typed,
                    signatures,
                    &types,
                    &proof.proposition,
                )
                .err()
            };
            if let Some(flaw) = flaw {
                let flaw = flaw.at_span_id(bind.name_span, &typed.span_table);
                let bind = typed.defs.get_mut(&id).unwrap();
                if !bind.flaws.contains(&flaw) {
                    bind.push_flaw(flaw);
                }
            }
        }
    }
}

fn payload_index(typed: &TypedFileAst, union: &Ty, payload: &Ty) -> Option<usize> {
    let Ty::Union { variants, .. } = typed.type_registry.resolved_definition_for_type(union) else {
        return None;
    };
    variants.iter().position(|variant| {
        variant.kind == ast::ty::UnionVariantKind::Type
            && matches!(variant.fields.as_slice(), [(_, ty)] if ty.as_ref() == payload)
    })
}

fn returned_payload(typed: &TypedFileAst, expression: ExprId, payload: &Ty) -> Option<ExprId> {
    let ty = typed.exprs.ty_of(expression)?;
    if ty == payload {
        return Some(expression);
    }
    let variant = payload_index(typed, ty, payload)?;
    if let Some(TypedExprKind::TagCall {
        variant_id, args, ..
    }) = typed.exprs.kind_of(expression)
    {
        let Ty::Union { variants, .. } = typed.type_registry.resolved_definition_for_type(ty)
        else {
            return None;
        };
        if variant_id.name != variants[variant].name {
            return None;
        }
        if let Some(args) = args
            && args.len() == 1
            && typed.exprs.ty_of(args[0]) == Some(payload)
        {
            return Some(args[0]);
        }
        return Some(expression);
    }
    Some(expression)
}

fn result_value(
    typed: &TypedFileAst,
    signatures: &CallableSignatureScope<'_>,
    expression: ExprId,
    payload: &Ty,
) -> Option<NormalExpr> {
    let ty = typed.exprs.ty_of(expression)?;
    let value = normal_expr(typed, Some(signatures), &HashMap::new(), expression);
    if ty == payload {
        return value.or_else(|| {
            Some(super::super::symbolic_values::call_result_symbol(
                expression,
            ))
        });
    }
    let variant = payload_index(typed, ty, payload)?;
    let NormalExpr::Var(root) = value
        .filter(|value| matches!(value, NormalExpr::Var(_)))
        .unwrap_or_else(|| super::super::symbolic_values::call_result_symbol(expression))
    else {
        unreachable!();
    };
    Some(NormalExpr::Var(payload_symbol(root, variant)))
}

impl WalkExprCtx<'_> {
    pub(super) fn remember_payload_pattern(
        &mut self,
        subject: ExprId,
        pattern: &ast::Pattern,
        places: &[PlaceId],
    ) {
        let ast::Pattern::Generic {
            name: variant_name, ..
        } = pattern
        else {
            return;
        };
        let Some(ty) = self.typed.exprs.ty_of(subject) else {
            return;
        };
        let Ty::Union { variants, .. } = self
            .typed
            .type_registry
            .resolved_definition_for_type(ty.without_reference_wrappers())
        else {
            return;
        };
        let Some((index, variant)) = variants
            .iter()
            .enumerate()
            .find(|(_, variant)| variant.name == *variant_name)
        else {
            return;
        };
        if variant.kind != ast::ty::UnionVariantKind::Type || variant.fields.len() != 1 {
            return;
        }
        let names = pattern.binding_names();
        let [name] = names.as_slice() else {
            return;
        };
        let ty = *variant.fields[0].1.clone();
        let Some(NormalExpr::Var(root)) = normal_expr(
            self.typed,
            Some(self.callable_signatures),
            &HashMap::new(),
            subject,
        ) else {
            return;
        };
        self.flow_ctx.binding_types.insert(*name, ty.clone());
        if let Some(place) = places
            .iter()
            .find(|place| self.typed.places[place.0 as usize].name == *name)
        {
            self.flow_ctx.record_targets.insert(
                *name,
                ReferenceTargetSet::singleton(ReferenceTargetGroup::Local(*place)),
            );
        }
        self.remember_record_value(*name, &ty, &NormalExpr::Var(payload_symbol(root, index)));
        super::record_constraints::assume_record_constraints(
            self.typed,
            self.callable_signatures,
            *name,
            &ty,
            &mut self.flow_ctx.constraint_env,
        );
    }

    pub(super) fn prepare_payload_inputs(&mut self) {
        let Some(bind) = self.definition.and_then(|id| self.typed.defs.get(&id)) else {
            return;
        };
        if bind.payload_postconditions.is_empty() {
            return;
        }
        let parameters: Vec<_> = bind
            .params
            .iter()
            .zip(&bind.param_conventions)
            .filter(|(_, convention)| {
                !matches!(
                    convention,
                    ParamConvention::Observe | ParamConvention::Mutate
                )
            })
            .map(|((name, ty), _)| (*name, ty.clone()))
            .collect();
        for (name, ty) in parameters {
            let snapshot = Intern::new(format!("$entry.{name}"));
            let value = NormalExpr::Var(name);
            if super::super::symbolic_values::scalar_type(self.typed, &ty) {
                self.flow_ctx
                    .constraint_env
                    .assume(&Predicate::Eq(NormalExpr::Var(snapshot), value));
            } else {
                self.remember_record_value(snapshot, &ty, &value);
            }
            self.payload_inputs.insert(name, NormalExpr::Var(snapshot));
        }
    }

    pub(super) fn verify_payload_postconditions(&mut self, expression: ExprId) {
        let Some(bind) = self.definition.and_then(|id| self.typed.defs.get(&id)) else {
            return;
        };
        let proofs = bind.payload_postconditions.clone();
        let parameters: HashMap<_, _> = bind.params.iter().cloned().collect();
        for proof in proofs.iter() {
            let Some(returned) = returned_payload(self.typed, expression, &proof.ty) else {
                continue;
            };
            let NormalExpr::Var(root) = super::super::symbolic_values::call_result_symbol(returned)
            else {
                unreachable!();
            };
            let root = if self.typed.exprs.ty_of(returned) == Some(&proof.ty) {
                root
            } else {
                payload_symbol(
                    root,
                    payload_index(
                        self.typed,
                        &self.typed.exprs.ty[returned.index()],
                        &proof.ty,
                    )
                    .unwrap(),
                )
            };
            self.remember_record_fields_as(root, returned, &proof.ty);
            let mut types = parameters.clone();
            types.insert(proof.name, proof.ty.clone());
            let mut values = self.payload_inputs.clone();
            if let Some(value) =
                result_value(self.typed, self.callable_signatures, returned, &proof.ty)
            {
                values.insert(proof.name, value);
            }
            let flaw = self.payload_postcondition_flaw(&types, values, proof);
            if let Some(flaw) = flaw {
                let flaws = &mut self.typed.exprs.facts.semantic_facts.flaws[expression.index()];
                if !flaws.contains(&flaw) {
                    flaws.push(flaw);
                }
            }
        }
    }

    fn payload_postcondition_flaw(
        &self,
        types: &HashMap<Intern<String>, Ty>,
        values: HashMap<Intern<String>, NormalExpr>,
        proof: &crate::typed::TypedPayloadPostcondition,
    ) -> Option<Diagnostic> {
        if let Err(flaw) = super::super::refinement_types::validate_proposition(
            self.typed,
            self.callable_signatures,
            types,
            &proof.proposition,
        ) {
            return Some(flaw);
        }
        let result = values
            .contains_key(&proof.name)
            .then(|| {
                super::postconditions::resolve(
                    self.typed,
                    self.callable_signatures,
                    types,
                    values,
                    &proof.proposition,
                )
            })
            .flatten()
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
                    "return contradicts payload postcondition `{}`",
                    proof.proposition
                ),
            )),
            ProveResult::Unknown => Some(refinement_diagnostics::postcondition(&proof.proposition)),
        }
    }

    pub(super) fn apply_payload_postconditions(
        &mut self,
        expression: ExprId,
        signature: &TypedCallableSignature,
        substitutions: &crate::subst::DepSubst,
        types: &HashMap<Intern<String>, Ty>,
        values: &HashMap<Intern<String>, NormalExpr>,
    ) {
        let result_ty = &self.typed.exprs.ty[expression.index()];
        let NormalExpr::Var(root) = super::super::symbolic_values::call_result_symbol(expression)
        else {
            unreachable!();
        };
        for proof in signature.payload_postconditions.iter() {
            let payload_ty = substitutions.apply_to_ty(&proof.ty);
            let value = if result_ty == &payload_ty {
                NormalExpr::Var(root)
            } else if let Some(variant) = payload_index(self.typed, result_ty, &payload_ty) {
                NormalExpr::Var(payload_symbol(root, variant))
            } else {
                continue;
            };
            let mut types = types.clone();
            let mut values = values.clone();
            types.insert(proof.name, payload_ty);
            values.insert(proof.name, value);
            let surface = ast::PredicateExpr::Proposition(Box::new(proof.proposition.clone()));
            if super::super::refinement_terms::predicate_names(&surface)
                .iter()
                .any(|name| {
                    let root =
                        Intern::from_ref(name.as_str().split('.').next().unwrap_or(name.as_str()));
                    !values.contains_key(&root)
                })
            {
                continue;
            }
            if let Some(predicate) = super::postconditions::resolve(
                self.typed,
                self.callable_signatures,
                &types,
                values,
                &proof.proposition,
            ) {
                self.flow_ctx.constraint_env.assume(&predicate);
            }
        }
    }
}
