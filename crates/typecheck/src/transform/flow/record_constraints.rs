use super::*;
use crate::transform::symbolic_values::{field_symbol, normal_expr};
use diagnostic::code::ProofCode;

fn has_field(expression: &NormalExpr) -> bool {
    match expression {
        NormalExpr::Var(name) => name.as_str().contains(".$field"),
        NormalExpr::Add(left, right)
        | NormalExpr::Sub(left, right)
        | NormalExpr::Mul(left, right) => has_field(left) || has_field(right),
        _ => false,
    }
}

pub(super) fn fields(typed: &TypedFileAst, ty: &Ty) -> Option<Vec<(Intern<String>, Box<Ty>)>> {
    let ty =
        reference_pointee_for_type(ty, Some(&typed.type_registry)).unwrap_or_else(|| ty.clone());
    match typed.type_registry.resolved_definition_for_type(&ty) {
        Ty::Record { fields, .. } => Some(fields.clone()),
        _ => None,
    }
}

fn refinements(typed: &TypedFileAst, ty: &Ty) -> HashMap<Intern<String>, ast::PredicateExpr> {
    typed
        .type_registry
        .declaration_for_type(ty)
        .map(|declaration| declaration.field_refinements.clone())
        .unwrap_or_default()
}

pub(super) fn assume_integer_bounds(
    typed: &TypedFileAst,
    ty: &Ty,
    value: NormalExpr,
    constraints: &mut ConstraintEnv,
) {
    if let Some((lower, upper)) = typed
        .type_registry
        .integer_validity_for_type(ty.without_reference_wrappers())
        .and_then(|validity| {
            validity.domain().closed_bounds().or_else(|| {
                validity
                    .domain()
                    .storage_hull()
                    .map(|hull| (NormalExpr::from(hull.min()), NormalExpr::from(hull.max())))
            })
        })
    {
        constraints.assume(&Predicate::Ge(value.clone(), lower));
        constraints.assume(&Predicate::Le(value.clone(), upper));
    }
    if let Some(validity) = typed
        .type_registry
        .integer_validity_for_type(ty.without_reference_wrappers())
    {
        match validity.domain().predicate() {
            ast::integer::CanonicalPredicate::Structured(predicate) => {
                constraints.assume(&predicate_expr_to_predicate(
                    predicate,
                    value.clone(),
                    &HashMap::from([(Intern::from_ref("self"), value)]),
                ));
            }
            ast::integer::CanonicalPredicate::LowerBound { bound, inclusive } => {
                constraints.assume(&if *inclusive {
                    Predicate::Ge(value, (*bound).into())
                } else {
                    Predicate::Gt(value, (*bound).into())
                });
            }
            ast::integer::CanonicalPredicate::UpperBound { bound, inclusive } => {
                constraints.assume(&if *inclusive {
                    Predicate::Le(value, (*bound).into())
                } else {
                    Predicate::Lt(value, (*bound).into())
                });
            }
            _ => {}
        }
    }
}

pub(super) fn record_predicate(
    typed: &TypedFileAst,
    signatures: &CallableSignatureScope<'_>,
    types: &HashMap<Intern<String>, Ty>,
    values: &HashMap<Intern<String>, NormalExpr>,
    subject: NormalExpr,
    refinement: &ast::PredicateExpr,
    owner: &Ty,
) -> Option<Predicate> {
    let mut values = values.clone();
    super::super::refinement_terms::extend_values(
        typed,
        signatures,
        types,
        &mut values,
        refinement,
    );
    let mut substitution = crate::subst::DepSubst::new();
    if let Some(instance) = owner.named_instance_stripping_reference_wrappers() {
        for (name, argument) in &instance.arguments {
            match argument {
                ast::TyArg::Type(ty) => {
                    substitution.types.insert(*name, *ty.clone());
                }
                ast::TyArg::Const(value) => {
                    substitution.consts.insert(*name, value.clone());
                }
            }
        }
    }
    super::super::refinement_terms::values_complete(refinement, &values).then(|| {
        resolve_target_queries_in_predicate(
            &predicate_expr_to_predicate(refinement, subject, &values),
            &substitution,
            typed,
        )
    })
}

pub(super) fn assume_record_constraints(
    typed: &TypedFileAst,
    signatures: &CallableSignatureScope<'_>,
    name: Intern<String>,
    ty: &Ty,
    constraints: &mut ConstraintEnv,
) {
    fn visit(
        typed: &TypedFileAst,
        signatures: &CallableSignatureScope<'_>,
        name: Intern<String>,
        ty: &Ty,
        constraints: &mut ConstraintEnv,
        depth: usize,
    ) {
        if depth == 16 {
            return;
        }
        let Some(fields) = fields(typed, ty) else {
            return;
        };
        let values: HashMap<_, _> = fields
            .iter()
            .enumerate()
            .map(|(index, (field, _))| (*field, field_symbol(name, index)))
            .collect();
        let types: HashMap<_, _> = fields
            .iter()
            .map(|(field, ty)| (*field, ty.as_ref().clone()))
            .collect();
        let refinements = refinements(typed, ty);
        let owner = ty;
        for (field, ty) in fields {
            let value = values[&field].clone();
            assume_integer_bounds(typed, &ty, value.clone(), constraints);
            if let Some(refinement) = refinements.get(&field)
                && let Some(predicate) = record_predicate(
                    typed,
                    signatures,
                    &types,
                    &values,
                    value.clone(),
                    refinement,
                    owner,
                )
            {
                constraints.assume(&predicate);
            }
            if !is_reference_type(typed, &ty)
                && let NormalExpr::Var(nested) = value
            {
                visit(typed, signatures, nested, &ty, constraints, depth + 1);
            }
        }
    }
    visit(typed, signatures, name, ty, constraints, 0);
}

impl WalkExprCtx<'_> {
    fn call_preserves_field_scalars(&self, argument: ExprId, field_ty: &Ty) -> bool {
        let Some(fields) = fields(self.typed, field_ty) else {
            return false;
        };
        let numeric: Vec<_> = fields
            .iter()
            .enumerate()
            .filter_map(|(index, (_, ty))| {
                self.typed
                    .type_registry
                    .integer_validity_for_type(ty)
                    .is_some()
                    .then_some(index)
            })
            .collect();
        if numeric.is_empty()
            || fields.iter().any(|(_, ty)| {
                matches!(
                    self.typed.type_registry.resolved_definition_for_type(ty),
                    Ty::Record { .. }
                )
            })
        {
            return false;
        }
        self.typed.exprs.kind.iter().any(|kind| {
            let TypedExprKind::FnCall {
                target,
                args: Some(args),
                ..
            } = kind
            else {
                return false;
            };
            let Some(slot) = args.iter().position(|arg| *arg == argument) else {
                return false;
            };
            let Some(signature) = self.callable_signatures.get(target) else {
                return false;
            };
            if signature.effects.unknown_memory {
                return false;
            }
            let Some((name, _)) = signature.params.get(slot) else {
                return false;
            };
            let Some(group) = signature.param_groups.get(slot).copied().flatten() else {
                return false;
            };
            let mut escapes = false;
            signature.references.result.visit(&mut |result| {
                for source in &result.sources {
                    match source {
                        crate::typed::ReferenceSource::Unknown => escapes = true,
                        crate::typed::ReferenceSource::Argument(reference)
                        | crate::typed::ReferenceSource::ArgumentElements(reference)
                            if reference.parameter == slot =>
                        {
                            escapes |= reference
                                .fields
                                .first()
                                .is_none_or(|index| numeric.contains(index));
                        }
                        crate::typed::ReferenceSource::Target(target) => {
                            escapes |= numeric.iter().any(|index| {
                                target.overlap(
                                    &ReferenceTargetGroup::Field {
                                        base: Box::new(ReferenceTargetGroup::Param(group)),
                                        index: *index,
                                    },
                                    &ConstraintEnv::default(),
                                ) != Overlap::Disjoint
                            });
                        }
                        _ => {}
                    }
                }
            });
            if escapes {
                return false;
            }
            numeric.iter().all(|index| {
                signature.effects.scalar_fields.iter().any(|effect| {
                    effect.parameter == slot
                        && effect.field.as_slice() == [*index]
                        && effect.value == field_symbol(*name, *index)
                })
            })
        })
    }

    pub(super) fn validate_constrained_field_borrow(&mut self, expression: ExprId, inner: ExprId) {
        let Some(TypedExprKind::AggregateGet { base, index }) = self.typed.exprs.kind_of(inner)
        else {
            return;
        };
        let ty = &self.typed.exprs.ty[base.index()];
        if super::dependent_arrays::field_has_array_dependency(self.typed, ty, *index) {
            self.reject_dependent_array_mutation(expression);
            return;
        }
        let refinements = refinements(self.typed, ty);
        let Some(fields) = fields(self.typed, ty) else {
            return;
        };
        let Some((field, field_ty)) = fields.get(*index) else {
            return;
        };
        fn mentions(predicate: &ast::PredicateExpr, field: Intern<String>) -> bool {
            fn term_mentions(term: &NormalExpr, field: Intern<String>) -> bool {
                match term {
                    NormalExpr::Var(name) => *name == field,
                    NormalExpr::Add(left, right)
                    | NormalExpr::Sub(left, right)
                    | NormalExpr::Mul(left, right) => {
                        term_mentions(left, field) || term_mentions(right, field)
                    }
                    _ => false,
                }
            }
            match predicate {
                ast::PredicateExpr::And(predicates) => predicates
                    .iter()
                    .any(|predicate| mentions(predicate, field)),
                ast::PredicateExpr::Lt(term)
                | ast::PredicateExpr::Gt(term)
                | ast::PredicateExpr::Le(term)
                | ast::PredicateExpr::Ge(term)
                | ast::PredicateExpr::Eq(term)
                | ast::PredicateExpr::Ne(term) => term_mentions(term, field),
                ast::PredicateExpr::Proposition(_) => {
                    super::super::refinement_terms::predicate_names(predicate)
                        .into_iter()
                        .any(|name| {
                            name == field || name.as_str().starts_with(&format!("{field}."))
                        })
                }
            }
        }
        let types: HashMap<_, _> = fields
            .iter()
            .map(|(name, ty)| (*name, ty.as_ref().clone()))
            .collect();
        let values = HashMap::new();
        let nested_fields = self::fields(self.typed, field_ty).unwrap_or_default();
        let only_scalar_dependencies = refinements.iter().all(|(subject, predicate)| {
            if subject == field && !matches!(predicate, ast::PredicateExpr::Proposition(_)) {
                return false;
            }
            super::super::refinement_terms::predicate_names(predicate)
                .into_iter()
                .all(|name| {
                    if name == *field {
                        return false;
                    }
                    if !name.as_str().starts_with(&format!("{field}.")) {
                        return true;
                    }
                    let Ok((_, Some(value))) = super::super::refinement_terms::resolve(
                        self.typed,
                        self.callable_signatures,
                        &types,
                        &values,
                        name,
                    ) else {
                        return false;
                    };
                    let mut dependencies = HashSet::new();
                    super::super::scalar_mutations::variables(&value, &mut dependencies);
                    dependencies.iter().all(|name| {
                        name.as_str()
                            .strip_prefix(&format!("{field}.$field"))
                            .and_then(|index| index.parse::<usize>().ok())
                            .and_then(|index| nested_fields.get(index))
                            .is_some_and(|(_, ty)| {
                                self.typed
                                    .type_registry
                                    .integer_validity_for_type(ty)
                                    .is_some()
                            })
                    })
                })
        });
        if only_scalar_dependencies && self.call_preserves_field_scalars(expression, field_ty) {
            return;
        }
        if refinements.contains_key(field)
            || refinements
                .values()
                .any(|predicate| mentions(predicate, *field))
        {
            self.typed.exprs.facts.semantic_facts.flaws[expression.index()].push(
                // dep-refinement-unproven
                Diagnostic::new(
                    ProofCode::DependentRefinementUnproven,
                    format!(
                        "mutable access to `{field}` must retain its parent record's constraints"
                    ),
                )
                .with_arg("name", field.as_str().to_string())
                .with_help(
                    "replace the field through its parent record after checking the affected \
                     constraints, instead of exposing a mutable field reference",
                ),
            );
        }
    }

    pub(super) fn remember_record_value(
        &mut self,
        name: Intern<String>,
        ty: &Ty,
        value: &NormalExpr,
    ) {
        match value {
            NormalExpr::Var(source) => self.copy_record_fields(name, *source, ty, 0),
            NormalExpr::Value(value) => self.remember_constant_fields(name, ty, value, 0),
            _ => {}
        }
    }

    pub(super) fn remember_returned_record(
        &mut self,
        expression: ExprId,
        captured: &[Option<NormalExpr>],
        signature: &TypedCallableSignature,
    ) {
        let Some(result) = &signature.effects.scalar_result else {
            return;
        };
        let ty = self.typed.exprs.ty[expression.index()].clone();
        if !matches!(
            self.typed.type_registry.resolved_definition_for_type(&ty),
            Ty::Record { .. }
        ) {
            return;
        }
        let arguments = signature
            .params
            .iter()
            .zip(captured)
            .filter_map(|((name, _), value)| value.clone().map(|value| (*name, value)))
            .collect();
        let Some(value) = super::super::scalar_mutations::instantiate(result, &arguments) else {
            return;
        };
        let NormalExpr::Var(root) = super::super::symbolic_values::call_result_symbol(expression)
        else {
            return;
        };
        self.remember_record_value(root, &ty, &value);
    }

    pub(super) fn remember_record_fields(&mut self, name: Intern<String>, body: ExprId) {
        let ty = self.typed.exprs.ty[body.index()].clone();
        self.remember_record_fields_as(name, body, &ty);
    }

    pub(super) fn remember_record_fields_as(
        &mut self,
        name: Intern<String>,
        body: ExprId,
        ty: &Ty,
    ) {
        let mut pending = vec![(name, body, ty.clone(), 0)];
        while let Some((name, body, ty, depth)) = pending.pop() {
            if depth == 16 {
                continue;
            }
            let Some(TypedExprKind::TagCall {
                args: Some(args),
                field_names,
                ..
            }) = self.typed.exprs.kind_of(body)
            else {
                if let Some(value) = normal_expr(
                    self.typed,
                    Some(self.callable_signatures),
                    &HashMap::new(),
                    body,
                ) {
                    self.remember_record_value(name, &ty, &value);
                }
                continue;
            };
            let Some(fields) = fields(self.typed, &ty) else {
                continue;
            };
            for (slot, argument) in args.iter().enumerate() {
                let index = field_names
                    .get(slot)
                    .and_then(|name| fields.iter().position(|(field, _)| field == name))
                    .unwrap_or(slot);
                let Some((_, field_ty)) = fields.get(index) else {
                    continue;
                };
                let symbol = field_symbol(name, index);
                if !is_reference_type(self.typed, field_ty)
                    && matches!(
                        self.typed
                            .type_registry
                            .resolved_definition_for_type(field_ty),
                        Ty::Record { .. }
                    )
                {
                    let NormalExpr::Var(nested) = symbol else {
                        unreachable!()
                    };
                    pending.push((nested, *argument, field_ty.as_ref().clone(), depth + 1));
                } else if let Some(value) = normal_expr(
                    self.typed,
                    Some(self.callable_signatures),
                    self.local_const_values,
                    *argument,
                ) && !has_field(&value)
                {
                    self.flow_ctx
                        .constraint_env
                        .assume(&Predicate::Eq(symbol, value));
                }
            }
        }
    }

    pub(super) fn remember_constant_fields(
        &mut self,
        name: Intern<String>,
        ty: &Ty,
        value: &ConstValue,
        depth: usize,
    ) {
        if depth == 16 {
            return;
        }
        let ConstValue::Record { fields: values } = value else {
            return;
        };
        let Some(fields) = fields(self.typed, ty) else {
            return;
        };
        for (index, ((_, ty), (_, value))) in fields.iter().zip(values.iter()).enumerate() {
            let symbol = field_symbol(name, index);
            if crate::transform::symbolic_values::scalar_type(self.typed, ty)
                && matches!(value, ConstValue::Int(_))
            {
                self.flow_ctx
                    .constraint_env
                    .assume(&Predicate::Eq(symbol, NormalExpr::Value(value.clone())));
            } else if !is_reference_type(self.typed, ty)
                && let NormalExpr::Var(nested) = symbol
            {
                self.remember_constant_fields(nested, ty, value, depth + 1);
            }
        }
    }

    pub(super) fn copy_record_fields(
        &mut self,
        destination: Intern<String>,
        source: Intern<String>,
        ty: &Ty,
        depth: usize,
    ) {
        if depth == 16 {
            return;
        }
        let Some(fields) = fields(self.typed, ty) else {
            if let Ty::Union { variants, .. } =
                self.typed.type_registry.resolved_definition_for_type(ty)
            {
                for (index, variant) in variants.iter().enumerate() {
                    if variant.kind == ast::ty::UnionVariantKind::Type && variant.fields.len() == 1
                    {
                        let left =
                            super::super::symbolic_values::payload_symbol(destination, index);
                        let right = super::super::symbolic_values::payload_symbol(source, index);
                        self.copy_record_fields(left, right, &variant.fields[0].1, depth + 1);
                    }
                }
            }
            return;
        };
        for (index, (_, field_ty)) in fields.iter().enumerate() {
            let left = field_symbol(destination, index);
            let right = field_symbol(source, index);
            if crate::transform::symbolic_values::scalar_type(self.typed, field_ty) {
                self.flow_ctx
                    .constraint_env
                    .assume(&Predicate::Eq(left.clone(), right.clone()));
            }
            if !is_reference_type(self.typed, field_ty)
                && let (NormalExpr::Var(left), NormalExpr::Var(right)) = (left, right)
            {
                self.copy_record_fields(left, right, field_ty, depth + 1);
            }
        }
    }

    pub(super) fn remember_destructured_fields(
        &mut self,
        value: ExprId,
        bindings: &[(Intern<String>, Intern<String>)],
        places: &[PlaceId],
    ) {
        let Some(ty) = self.typed.exprs.ty_of(value) else {
            return;
        };
        let Some(fields) = fields(self.typed, ty) else {
            return;
        };
        let source = normal_expr(
            self.typed,
            Some(self.callable_signatures),
            &HashMap::new(),
            value,
        );
        for (field, name) in bindings {
            let Some((index, (_, ty))) = fields
                .iter()
                .enumerate()
                .find(|(_, (member, _))| member == field)
            else {
                continue;
            };
            let Some(place) = places
                .iter()
                .copied()
                .find(|place| self.typed.places[place.0 as usize].name == *name)
            else {
                continue;
            };
            self.flow_ctx
                .binding_types
                .insert(*name, self.typed.places[place.0 as usize].ty.clone());
            let targets = if self.typed.borrows_value(value) {
                self.record_targets_for(value).map(|targets| {
                    targets.projected(|base| ReferenceTargetGroup::Field {
                        base: Box::new(base.clone()),
                        index,
                    })
                })
            } else {
                Some(ReferenceTargetSet::singleton(ReferenceTargetGroup::Local(
                    place,
                )))
            };
            self.flow_ctx.record_targets.remove(name);
            if let Some(targets) = targets {
                self.flow_ctx.record_targets.insert(*name, targets);
            }
            if crate::transform::symbolic_values::scalar_type(self.typed, ty) {
                self.flow_ctx.scalar_bindings.insert(*name);
            }
            if let Some(NormalExpr::Var(root)) = &source {
                let projected = field_symbol(*root, index);
                if crate::transform::symbolic_values::scalar_type(self.typed, ty) {
                    self.flow_ctx
                        .constraint_env
                        .assume(&Predicate::Eq(NormalExpr::Var(*name), projected));
                } else if let NormalExpr::Var(root) = projected {
                    self.copy_record_fields(*name, root, ty, 0);
                }
            }
            assume_record_constraints(
                self.typed,
                self.callable_signatures,
                *name,
                ty,
                &mut self.flow_ctx.constraint_env,
            );
        }
    }

    pub(super) fn changes_record_facts(&self, expression: ExprId) -> bool {
        let mut pending = vec![expression];
        let mut visited = HashSet::new();
        while let Some(expression) = pending.pop() {
            if !visited.insert(expression) {
                continue;
            }
            match self.typed.exprs.kind_of(expression) {
                Some(
                    TypedExprKind::Reassign { .. }
                    | TypedExprKind::RecordSet { .. }
                    | TypedExprKind::AggregateSet { .. }
                    | TypedExprKind::BufSet { .. }
                    | TypedExprKind::Emit { .. },
                ) => return true,
                Some(TypedExprKind::FnCall { target, .. })
                    if self
                        .callable_signatures
                        .get(target)
                        .is_some_and(|signature| {
                            !signature.effects.writes.is_empty() || signature.effects.unknown_memory
                        }) =>
                {
                    return true;
                }
                _ => {}
            }
            let _ = crate::typed::walk_expr_children_of(self.typed, expression, &mut |child| {
                pending.push(child);
                std::ops::ControlFlow::Continue(())
            });
        }
        false
    }

    pub(super) fn forget_record_facts(&mut self) {
        let constraints = &mut self.flow_ctx.constraint_env;
        constraints
            .known_eq
            .retain(|(left, right)| !has_field(left) && !has_field(right));
        constraints
            .known_lt
            .retain(|(left, right)| !has_field(left) && !has_field(right));
        constraints
            .known_le
            .retain(|(left, right)| !has_field(left) && !has_field(right));
        constraints
            .known_divisible
            .retain(|(left, right)| !has_field(left) && !has_field(right));
        self.local_const_values
            .retain(|_, value| !matches!(value, ConstValue::Record { .. }));
    }

    pub(super) fn validate_record_constraints(&mut self, expression: ExprId, value: ExprId) {
        let (base, index) = match self.typed.exprs.kind_of(expression) {
            Some(TypedExprKind::AggregateSet { base, index, .. }) => (*base, *index),
            Some(TypedExprKind::RecordSet { base, field, .. }) => {
                let Some(index) = record_field_index(self.typed, *base, *field) else {
                    return;
                };
                (*base, index)
            }
            _ => return,
        };
        let ty = self.typed.exprs.ty[base.index()].clone();
        if super::dependent_arrays::field_has_array_dependency(self.typed, &ty, index) {
            self.reject_dependent_array_mutation(expression);
            return;
        }
        let refinements = refinements(self.typed, &ty);
        if refinements.is_empty() {
            return;
        }
        let Some(fields) = fields(self.typed, &ty) else {
            return;
        };
        let root = normal_expr(
            self.typed,
            Some(self.callable_signatures),
            &HashMap::new(),
            base,
        );
        let root = match root {
            Some(NormalExpr::Var(root)) => root,
            _ => Intern::new(format!("$record{}", base.0)),
        };
        let mut current: HashMap<_, _> = fields
            .iter()
            .enumerate()
            .map(|(index, (name, _))| (*name, field_symbol(root, index)))
            .collect();
        if let Some(NormalExpr::Value(ConstValue::Record { fields })) = normal_expr(
            self.typed,
            Some(self.callable_signatures),
            self.local_const_values,
            base,
        ) {
            for (name, value) in fields.iter() {
                current.insert(*name, NormalExpr::Value(value.clone()));
            }
        }
        let types: HashMap<_, _> = fields
            .iter()
            .map(|(name, ty)| (*name, ty.as_ref().clone()))
            .collect();
        let mut constraints = self.flow_ctx.constraint_env.clone();
        for (name, field_ty) in &fields {
            let field = current[name].clone();
            assume_integer_bounds(self.typed, field_ty, field.clone(), &mut constraints);
            if let Some(refinement) = refinements.get(name)
                && let Some(predicate) = record_predicate(
                    self.typed,
                    self.callable_signatures,
                    &types,
                    &current,
                    field,
                    refinement,
                    &ty,
                )
            {
                constraints.assume(&predicate);
            }
        }
        let replacement = normal_expr(
            self.typed,
            Some(self.callable_signatures),
            self.local_const_values,
            value,
        )
        .unwrap_or_else(|| NormalExpr::Var(Intern::new(format!("$value{}", value.0))));
        assume_integer_bounds(
            self.typed,
            &self.typed.exprs.ty[value.index()],
            replacement.clone(),
            &mut constraints,
        );
        let Some((changed, _)) = fields.get(index) else {
            return;
        };
        current.insert(*changed, replacement.clone());
        for (name, refinement) in &refinements {
            if name != changed
                && !super::super::refinement_terms::predicate_names(refinement)
                    .iter()
                    .any(|name| name.as_str().split('.').next() == Some(changed.as_str()))
            {
                continue;
            }
            let Some(field) = current.get(name) else {
                continue;
            };
            let predicate = record_predicate(
                self.typed,
                self.callable_signatures,
                &types,
                &current,
                field.clone(),
                refinement,
                &ty,
            );
            let proof = predicate.map_or(ProveResult::Unknown, |predicate| {
                constraints.prove(&predicate, &HashMap::new())
            });
            let flaw = match proof {
                ProveResult::Proven => continue,
                // dep-refinement-failed
                ProveResult::Disproven => Diagnostic::new(
                    ProofCode::DependentRefinementFailed,
                    format!("replacement of `{changed}` must preserve constraint on `{name}`"),
                )
                .with_arg("name", name.as_str().to_string()),
                ProveResult::Unknown => {
                    refinement_diagnostics::replacement(changed.as_str(), name.as_str(), refinement)
                }
            };
            self.typed.exprs.facts.semantic_facts.flaws[expression.index()].push(flaw);
        }
    }
}
