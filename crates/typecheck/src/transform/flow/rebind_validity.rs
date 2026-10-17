use super::*;
use crate::normal_expr::Normalize;
use ast::integer::{CanonicalPredicate, ContainmentProof, IntegerDomain, IntegerKnowledge};

impl WalkExprCtx<'_> {
    pub(super) fn rebind_preserves_integer_validity(
        &self,
        target: Option<&Ty>,
        expression: ExprId,
        value: Option<&NormalExpr>,
        previous: &NormalExpr,
        interval_proven: bool,
    ) -> bool {
        let Some(expected) = target
            .and_then(|ty| self.typed.type_registry.integer_validity_for_type(ty))
            .map(|validity| validity.domain())
        else {
            return true;
        };
        if let Some(Some(ConstValue::Int(value))) = self.typed.exprs.const_value_of(expression) {
            return expected.membership(*value) == ContainmentProof::Proven;
        }
        if let Some(value) = value.and_then(integer_value) {
            return expected.membership(value) == ContainmentProof::Proven;
        }
        let knowledge = self
            .typed
            .exprs
            .integer_knowledge_of(expression)
            .and_then(Option::as_ref);
        if matches!(knowledge, Some(IntegerKnowledge::Poison)) {
            return false;
        }
        let actual = knowledge
            .and_then(super::super::place::knowledge_domain)
            .or_else(|| {
                self.typed
                    .exprs
                    .ty_of(expression)
                    .and_then(|ty| self.typed.type_registry.integer_validity_for_type(ty))
                    .map(|validity| validity.domain().clone())
            });
        if actual
            .as_ref()
            .is_some_and(|actual| actual.prove_containment(expected) == ContainmentProof::Proven)
        {
            return true;
        }
        let Some(value) = value else {
            return false;
        };
        let values = self
            .local_const_values
            .iter()
            .map(|(name, value)| (*name, NormalExpr::Value(value.clone())))
            .collect();
        IntegerRebindProof {
            expected,
            value,
            previous,
            interval_proven,
            constraints: &self.flow_ctx.constraint_env,
            values: &values,
        }
        .proves_domain(expected.predicate())
    }
}

struct IntegerRebindProof<'a> {
    expected: &'a IntegerDomain,
    value: &'a NormalExpr,
    previous: &'a NormalExpr,
    interval_proven: bool,
    constraints: &'a ConstraintEnv,
    values: &'a HashMap<Intern<String>, NormalExpr>,
}

impl IntegerRebindProof<'_> {
    fn proves_domain(&self, predicate: &CanonicalPredicate) -> bool {
        match predicate {
            CanonicalPredicate::Empty | CanonicalPredicate::Symbolic(_) => false,
            CanonicalPredicate::Unbounded => true,
            CanonicalPredicate::Hull(hull) => self.proves_interval(hull.min(), hull.max()),
            CanonicalPredicate::HullExcluding { hull, excluded } => {
                self.proves_interval(hull.min(), hull.max())
                    && excluded.iter().all(|value| {
                        self.proves_relation(&Predicate::Ne(self.value.clone(), (*value).into()))
                    })
            }
            CanonicalPredicate::LowerBound { bound, inclusive } => {
                self.proves_relation(&if *inclusive {
                    Predicate::Ge(self.value.clone(), (*bound).into())
                } else {
                    Predicate::Gt(self.value.clone(), (*bound).into())
                })
            }
            CanonicalPredicate::UpperBound { bound, inclusive } => {
                self.proves_relation(&if *inclusive {
                    Predicate::Le(self.value.clone(), (*bound).into())
                } else {
                    Predicate::Lt(self.value.clone(), (*bound).into())
                })
            }
            CanonicalPredicate::Structured(predicate) => self.proves_refinement(predicate),
            CanonicalPredicate::Union(predicates) => predicates
                .iter()
                .any(|predicate| self.proves_domain(predicate)),
        }
    }

    fn proves_interval(&self, min: I256, max: I256) -> bool {
        self.proves_relation(&Predicate::Ge(self.value.clone(), min.into()))
            && self.proves_relation(&Predicate::Le(self.value.clone(), max.into()))
    }

    fn proves_relation(&self, predicate: &Predicate) -> bool {
        if self.constraints.prove(predicate, self.values) == ProveResult::Proven {
            return true;
        }
        if !self.interval_proven {
            return false;
        }
        let Some(hull) = self.expected.storage_hull() else {
            return false;
        };
        match predicate {
            Predicate::Ge(value, bound) if value == self.value => {
                integer_value(bound).is_some_and(|bound| hull.min() >= bound)
            }
            Predicate::Gt(value, bound) if value == self.value => {
                integer_value(bound).is_some_and(|bound| hull.min() > bound)
            }
            Predicate::Le(value, bound) if value == self.value => {
                integer_value(bound).is_some_and(|bound| hull.max() <= bound)
            }
            Predicate::Lt(value, bound) if value == self.value => {
                integer_value(bound).is_some_and(|bound| hull.max() < bound)
            }
            Predicate::Ne(value, bound) if value == self.value => {
                integer_value(bound).is_some_and(|bound| bound < hull.min() || bound > hull.max())
            }
            Predicate::Eq(value, bound) if value == self.value => {
                integer_value(bound).is_some_and(|bound| hull.min() == bound && hull.max() == bound)
            }
            _ => false,
        }
    }

    fn proves_refinement(&self, refinement: &ast::PredicateExpr) -> bool {
        if let ast::PredicateExpr::And(predicates) = refinement {
            return predicates
                .iter()
                .all(|predicate| self.proves_refinement(predicate));
        }
        let predicate = predicate_expr_to_predicate(refinement, self.value.clone(), self.values);
        if self.proves_relation(&predicate) {
            return true;
        }
        let Some(hull) = self.expected.storage_hull() else {
            return false;
        };
        let dividend = hull.max();
        self.interval_proven
            && hull.min() > I256::from(0)
            && (dividend & (dividend - I256::from(1))) == I256::from(0)
            && refinement_divides_storage_max(self.expected)
            && proposition_divides_value(refinement, dividend)
            && matches!(self.value, NormalExpr::Mul(left, right)
                if left.as_ref() == self.previous && right.as_const_int() == Some(2))
    }
}

fn integer_value(expression: &NormalExpr) -> Option<I256> {
    match expression.normalize_to_value()? {
        ConstValue::Int(value) => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_membership_does_not_prove_an_exclusion() {
        let uncertain = IntegerDomain::from_refinement(&ast::PredicateExpr::And(vec![
            ast::PredicateExpr::Ge(1.into()),
            ast::PredicateExpr::Le(4.into()),
            ast::PredicateExpr::Ne(NormalExpr::Var(Intern::from_ref("unknown"))),
        ]));
        let actual = IntegerDomain::union([
            uncertain,
            IntegerDomain::bounded(8.into(), 8.into()).unwrap(),
        ]);
        let expected = IntegerDomain::excluding(1.into(), 8.into(), vec![2.into()]).unwrap();
        assert_eq!(actual.membership(2.into()), ContainmentProof::Unknown);
        assert_eq!(
            actual.prove_containment(&expected),
            ContainmentProof::Unknown
        );
    }

    #[test]
    fn an_interval_proof_does_not_fill_the_gaps_in_a_union() {
        let expected = IntegerDomain::union([
            IntegerDomain::bounded((-4).into(), (-2).into()).unwrap(),
            IntegerDomain::bounded(2.into(), 4.into()).unwrap(),
        ]);
        let value = NormalExpr::Var(Intern::from_ref("value"));
        for (min, max, valid) in [(-1, 1, false), (-4, -2, true), (2, 4, true)] {
            let mut constraints = ConstraintEnv::default();
            constraints.assume(&Predicate::Ge(value.clone(), min.into()));
            constraints.assume(&Predicate::Le(value.clone(), max.into()));
            let proof = IntegerRebindProof {
                expected: &expected,
                value: &value,
                previous: &value,
                interval_proven: true,
                constraints: &constraints,
                values: &HashMap::new(),
            };
            assert_eq!(proof.proves_domain(expected.predicate()), valid);
        }
    }

    #[test]
    fn a_union_does_not_give_every_old_value_a_divisor_predicate() {
        let divisor = IntegerDomain::from_refinement(&ast::PredicateExpr::And(vec![
            ast::PredicateExpr::Ge(1.into()),
            ast::PredicateExpr::Le(16.into()),
            ast::PredicateExpr::Proposition(Box::new(ast::ProofProposition::Compare {
                left: ast::ProofTerm::Remainder(
                    Box::new(ast::ProofTerm::Value(16.into())),
                    Box::new(ast::ProofTerm::Name(Intern::from_ref("self"))),
                ),
                relation: ast::ProofRelation::Equal,
                right: ast::ProofTerm::Value(0.into()),
            })),
        ]));
        let expected =
            IntegerDomain::union([divisor, IntegerDomain::bounded(3.into(), 3.into()).unwrap()]);
        let old = NormalExpr::Var(Intern::from_ref("old"));
        let value = NormalExpr::Mul(Box::new(old.clone()), Box::new(2.into()));
        let mut constraints = ConstraintEnv::default();
        constraints.assume(&Predicate::Eq(old.clone(), 3.into()));
        let proof = IntegerRebindProof {
            expected: &expected,
            value: &value,
            previous: &old,
            interval_proven: true,
            constraints: &constraints,
            values: &HashMap::new(),
        };
        assert!(!proof.proves_domain(expected.predicate()));
    }
}
