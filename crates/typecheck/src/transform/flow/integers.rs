use super::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::{IntegerCode, TypeCode};

impl WalkExprCtx<'_> {
    pub(super) fn validate_pointer_integer_refinement(
        &mut self,
        expression: ExprId,
        operand: ExprId,
    ) {
        let Some(source) = self.typed.exprs.ty_of(operand) else {
            return;
        };
        if !super::super::symbolic_values::address_type(self.typed, source) {
            return;
        }
        let target = &self.typed.exprs.ty[expression.index()];
        let Some(validity) = self.typed.type_registry.integer_validity_for_type(target) else {
            return;
        };
        let Some(width) = self.typed.type_registry.integer_width_for_type(target) else {
            return;
        };
        let Some(hull) = validity.domain().storage_hull() else {
            return;
        };
        let signed = hull.min() < 0.into();
        let boundary = I256::from(1) << (width - u32::from(signed));
        let minimum = if signed { -boundary } else { I256::from(0) };
        let maximum = boundary - I256::from(1);
        let physical = ast::integer::IntegerDomain::bounded(minimum, maximum).unwrap();
        if physical.prove_containment(validity.domain()) == ast::integer::ContainmentProof::Proven {
            return;
        }
        // type-integer-narrowing-unproven
        self.typed.exprs.facts.semantic_facts.flaws[expression.index()].push(Diagnostic::new(
            IntegerCode::NarrowingUnproven,
            "raw address conversion must prove the target integer refinement",
        ));
    }

    pub(super) fn validate_proven_integer_cast(&mut self, expression: ExprId, operand: ExprId) {
        if !self.typed.exprs.facts.semantic_facts.flaws[expression.index()]
            .iter()
            .any(|flaw| {
                matches!(
                    flaw.code,
                    DiagnosticCode::Type(TypeCode::Integer(IntegerCode::NarrowingFailed))
                        | DiagnosticCode::Type(TypeCode::Integer(IntegerCode::NarrowingUnproven))
                )
            })
        {
            return;
        }
        let Some(source) = self.typed.exprs.ty_of(operand) else {
            return;
        };
        let source_address = super::super::symbolic_values::address_type(self.typed, source);
        if (!source_address
            && crate::discriminant::conversion_domain(&self.typed.type_registry, source).is_none())
            || matches!(
                self.typed.exprs.integer_knowledge_of(operand),
                Some(Some(ast::integer::IntegerKnowledge::Poison))
            )
        {
            return;
        }
        let Some(domain) = crate::discriminant::conversion_domain(
            &self.typed.type_registry,
            &self.typed.exprs.ty[expression.index()],
        ) else {
            return;
        };
        let Some(value) = super::super::symbolic_values::normal_expr(
            self.typed,
            Some(self.callable_signatures),
            self.local_const_values,
            operand,
        ) else {
            return;
        };
        let mut constraints = self.flow_ctx.constraint_env.clone();
        if source_address
            && !assume_address_bounds(
                self.typed,
                &self.typed.exprs.ty[expression.index()],
                &value,
                &mut constraints,
            )
        {
            return;
        }
        record_constraints::assume_integer_bounds(
            self.typed,
            source,
            value.clone(),
            &mut constraints,
        );
        self.assume_target_constant_bounds(&mut constraints);
        if integer_predicate_proven(domain.predicate(), &value, &constraints) {
            self.typed.exprs.facts.semantic_facts.flaws[expression.index()].retain(|flaw| {
                !matches!(
                    flaw.code,
                    DiagnosticCode::Type(TypeCode::Integer(IntegerCode::NarrowingFailed))
                        | DiagnosticCode::Type(TypeCode::Integer(IntegerCode::NarrowingUnproven))
                )
            });
        }
    }

    fn assume_target_constant_bounds(&self, constraints: &mut ConstraintEnv) {
        if self.typed.target_layout.is_some() {
            return;
        }
        let mut pending: Vec<_> = constraints
            .known_eq
            .iter()
            .chain(&constraints.known_lt)
            .chain(&constraints.known_le)
            .chain(&constraints.known_divisible)
            .flat_map(|(left, right)| [left, right])
            .collect();
        let mut names = Vec::new();
        while let Some(expression) = pending.pop() {
            match expression {
                NormalExpr::Var(name) => names.push(*name),
                NormalExpr::Add(left, right)
                | NormalExpr::Sub(left, right)
                | NormalExpr::Mul(left, right) => {
                    pending.extend([left.as_ref(), right.as_ref()]);
                }
                _ => {}
            }
        }
        names.sort();
        names.dedup();
        for name in names {
            let root = Intern::from_ref(name.as_str().split('.').next().unwrap_or(name.as_str()));
            if self.flow_ctx.binding_types.contains_key(&root) {
                continue;
            }
            if let Some((min, max)) =
                super::super::refinement_terms::constant_bounds(self.typed, name)
            {
                constraints.assume(&Predicate::Ge(NormalExpr::Var(name), min.into()));
                constraints.assume(&Predicate::Le(NormalExpr::Var(name), max.into()));
            }
        }
    }
}

fn integer_predicate_proven(
    domain: &ast::integer::CanonicalPredicate,
    value: &NormalExpr,
    constraints: &ConstraintEnv,
) -> bool {
    use ast::integer::CanonicalPredicate as D;
    let check = |predicate| constraints.prove(&predicate, &HashMap::new()) == ProveResult::Proven;
    match domain {
        D::Unbounded => true,
        D::Hull(hull) | D::HullExcluding { hull, .. } => {
            check(Predicate::Ge(value.clone(), hull.min().into()))
                && check(Predicate::Le(value.clone(), hull.max().into()))
                && match domain {
                    D::HullExcluding { excluded, .. } => excluded
                        .iter()
                        .all(|excluded| check(Predicate::Ne(value.clone(), (*excluded).into()))),
                    _ => true,
                }
        }
        D::LowerBound { bound, inclusive } => check(if *inclusive {
            Predicate::Ge(value.clone(), (*bound).into())
        } else {
            Predicate::Gt(value.clone(), (*bound).into())
        }),
        D::UpperBound { bound, inclusive } => check(if *inclusive {
            Predicate::Le(value.clone(), (*bound).into())
        } else {
            Predicate::Lt(value.clone(), (*bound).into())
        }),
        D::Union(domains) => domains
            .iter()
            .any(|domain| integer_predicate_proven(domain, value, constraints)),
        D::Structured(predicate) => check(predicate_expr_to_predicate(
            predicate,
            value.clone(),
            &HashMap::from([(Intern::from_ref("self"), value.clone())]),
        )),
        D::Empty | D::Symbolic(_) => false,
    }
}

fn assume_address_bounds(
    typed: &TypedFileAst,
    target: &Ty,
    value: &NormalExpr,
    constraints: &mut ConstraintEnv,
) -> bool {
    let Some(layout) = typed.target_layout else {
        return false;
    };
    let Some(width) = typed.type_registry.integer_width_for_type(target) else {
        return false;
    };
    let Some(hull) = typed
        .type_registry
        .integer_validity_for_type(target)
        .and_then(|validity| validity.domain().storage_hull())
    else {
        return false;
    };
    if u64::from(width) < layout.pointer_bits() || hull.min() < 0.into() {
        return false;
    }
    constraints.assume(&Predicate::Ge(value.clone(), 0_i128.into()));
    let maximum = (I256::from(1) << layout.pointer_bits()) - I256::from(1);
    constraints.assume(&Predicate::Le(value.clone(), maximum.into()));
    true
}
