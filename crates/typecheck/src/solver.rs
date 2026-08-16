use std::collections::HashMap;

use ast::{NormalExpr, PredicateExpr};
use internment::Intern;

use crate::normal_expr::Normalize;
use crate::subst::DepSubst;

/// A fully-resolved predicate with both sides explicit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    Lt(NormalExpr, NormalExpr),
    Gt(NormalExpr, NormalExpr),
    Le(NormalExpr, NormalExpr),
    Ge(NormalExpr, NormalExpr),
    Eq(NormalExpr, NormalExpr),
    Ne(NormalExpr, NormalExpr),
    And(Vec<Predicate>),
    Opaque(ast::ProofProposition),
}

/// Known constraints on const variables, used to resolve predicate checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConstraintEnv {
    pub known_eq: Vec<(NormalExpr, NormalExpr)>,
    pub known_lt: Vec<(NormalExpr, NormalExpr)>,
    pub known_le: Vec<(NormalExpr, NormalExpr)>,
}

impl ConstraintEnv {
    pub fn assume(&mut self, predicate: &Predicate) {
        match predicate {
            Predicate::Lt(left, right) => self.known_lt.push((left.clone(), right.clone())),
            Predicate::Gt(left, right) => self.known_lt.push((right.clone(), left.clone())),
            Predicate::Le(left, right) => self.known_le.push((left.clone(), right.clone())),
            Predicate::Ge(left, right) => self.known_le.push((right.clone(), left.clone())),
            Predicate::Eq(left, right) => self.known_eq.push((left.clone(), right.clone())),
            Predicate::Ne(_, _) => {}
            Predicate::Opaque(_) => {}
            Predicate::And(predicates) => {
                for predicate in predicates {
                    self.assume(predicate);
                }
            }
        }
    }

    /// Check whether a predicate holds given the current constraint environment.
    ///
    /// Variables in the predicate are looked up in `var_values` first — if a
    /// variable has a concrete value, it's substituted before checking.
    /// Falls through to `Unknown` when the info is insufficient.
    pub fn prove(
        &self,
        pred: &Predicate,
        var_values: &HashMap<Intern<String>, NormalExpr>,
    ) -> ProveResult {
        let subst = DepSubst {
            types: HashMap::new(),
            consts: var_values.clone(),
        };
        self.prove_with_substitution(pred, &subst)
    }

    pub fn prove_with_substitution(&self, pred: &Predicate, subst: &DepSubst) -> ProveResult {
        match pred {
            Predicate::Lt(l, r) => self.prove_lt(l, r, subst),
            Predicate::Gt(l, r) => self.prove_lt(r, l, subst),
            Predicate::Le(l, r) => self.prove_le(l, r, subst),
            Predicate::Ge(l, r) => self.prove_le(r, l, subst),
            Predicate::Eq(l, r) => self.prove_eq(l, r, subst),
            Predicate::Ne(l, r) => match self.prove_eq(l, r, subst) {
                ProveResult::Proven => ProveResult::Disproven,
                ProveResult::Disproven => ProveResult::Proven,
                ProveResult::Unknown => ProveResult::Unknown,
            },
            Predicate::And(preds) => {
                let mut result = ProveResult::Proven;
                for p in preds {
                    let r = self.prove_with_substitution(p, subst);
                    if r == ProveResult::Disproven {
                        return ProveResult::Disproven;
                    }
                    if r == ProveResult::Unknown {
                        result = ProveResult::Unknown;
                    }
                }
                result
            }
            Predicate::Opaque(_) => ProveResult::Unknown,
        }
    }

    fn prove_lt(&self, l: &NormalExpr, r: &NormalExpr, subst: &DepSubst) -> ProveResult {
        let l = subst.apply_to_normal(l).normalize();
        let r = subst.apply_to_normal(r).normalize();

        // Check literal comparison
        if let (NormalExpr::Value(a), NormalExpr::Value(b)) = (&l, &r) {
            if a.as_const_size_int()
                .zip(b.as_const_size_int())
                .is_some_and(|(a, b)| a < b)
            {
                return ProveResult::Proven;
            }
            return ProveResult::Disproven;
        }

        // Check known_lt facts in the constraint env
        if self.known_lt.iter().any(|(kl, kr)| &l == kl && &r == kr) {
            return ProveResult::Proven;
        }

        let mut frontier = vec![(l.clone(), false)];
        let mut visited = Vec::new();
        while let Some((current, strict)) = frontier.pop() {
            if visited.contains(&(current.clone(), strict)) {
                continue;
            }
            visited.push((current.clone(), strict));
            for (left, right) in &self.known_lt {
                if *left == current {
                    if *right == r {
                        return ProveResult::Proven;
                    }
                    frontier.push((right.clone(), true));
                }
            }
            for (left, right) in &self.known_le {
                if *left == current {
                    if strict && *right == r {
                        return ProveResult::Proven;
                    }
                    frontier.push((right.clone(), strict));
                }
            }
        }

        ProveResult::Unknown
    }

    fn prove_eq(&self, l: &NormalExpr, r: &NormalExpr, subst: &DepSubst) -> ProveResult {
        let l = subst.apply_to_normal(l).normalize();
        let r = subst.apply_to_normal(r).normalize();

        // Check literal comparison
        match (&l, &r) {
            (NormalExpr::Value(a), NormalExpr::Value(b)) if a == b => return ProveResult::Proven,
            (NormalExpr::Value(_), NormalExpr::Value(_)) => return ProveResult::Disproven,
            (NormalExpr::Inferred(a), NormalExpr::Inferred(b)) if a != b => {
                return ProveResult::Disproven;
            }
            _ if l == r => return ProveResult::Proven,
            _ => {}
        }

        // Check known_eq facts in the constraint env
        if self.known_eq.iter().any(|(kl, kr)| &l == kl && &r == kr) {
            return ProveResult::Proven;
        }

        ProveResult::Unknown
    }

    fn prove_le(&self, l: &NormalExpr, r: &NormalExpr, subst: &DepSubst) -> ProveResult {
        let l = subst.apply_to_normal(l).normalize();
        let r = subst.apply_to_normal(r).normalize();
        if self
            .known_le
            .iter()
            .any(|(known_left, known_right)| l == *known_left && r == *known_right)
        {
            return ProveResult::Proven;
        }
        self.prove_lt(&l, &r, subst)
            .or(self.prove_eq(&l, &r, subst))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProveResult {
    Proven,
    Disproven,
    Unknown,
}

impl ProveResult {
    /// Combine with OR semantics: Proven wins, then Unknown, then Disproven.
    fn or(self, other: ProveResult) -> ProveResult {
        match (self, other) {
            (ProveResult::Proven, _) | (_, ProveResult::Proven) => ProveResult::Proven,
            (ProveResult::Unknown, _) | (_, ProveResult::Unknown) => ProveResult::Unknown,
            _ => ProveResult::Disproven,
        }
    }
}

/// Convert a parse-level `PredicateExpr` to a resolved `Predicate` with the field value
/// as the explicit left-hand side. Variables in the right-hand side are resolved from
/// `var_values`.
pub fn predicate_expr_to_predicate(
    expr: &PredicateExpr,
    field_value: NormalExpr,
    var_values: &HashMap<Intern<String>, NormalExpr>,
) -> Predicate {
    let resolve = |rhs: &NormalExpr| -> NormalExpr {
        match rhs {
            NormalExpr::Var(name) => var_values.get(name).cloned().unwrap_or(rhs.clone()),
            _ => rhs.clone(),
        }
    };
    match expr {
        PredicateExpr::Lt(rhs) => Predicate::Lt(field_value, resolve(rhs)),
        PredicateExpr::Gt(rhs) => Predicate::Gt(field_value, resolve(rhs)),
        PredicateExpr::Le(rhs) => Predicate::Le(field_value, resolve(rhs)),
        PredicateExpr::Ge(rhs) => Predicate::Ge(field_value, resolve(rhs)),
        PredicateExpr::Eq(rhs) => Predicate::Eq(field_value, resolve(rhs)),
        PredicateExpr::Ne(rhs) => Predicate::Ne(field_value, resolve(rhs)),
        PredicateExpr::And(preds) => Predicate::And(
            preds
                .iter()
                .map(|p| predicate_expr_to_predicate(p, field_value.clone(), var_values))
                .collect(),
        ),
        PredicateExpr::Proposition(proposition) => {
            proposition_to_predicate(proposition, &field_value, var_values)
                .unwrap_or_else(|| Predicate::Opaque(proposition.as_ref().clone()))
        }
    }
}

fn proposition_to_predicate(
    proposition: &ast::ProofProposition,
    field_value: &NormalExpr,
    var_values: &HashMap<Intern<String>, NormalExpr>,
) -> Option<Predicate> {
    use ast::{ProofProposition, ProofRelation};
    match proposition {
        ProofProposition::Compare {
            left,
            relation,
            right,
        } => {
            let left = proof_term_to_normal(left, field_value, var_values)?;
            let right = proof_term_to_normal(right, field_value, var_values)?;
            Some(match relation {
                ProofRelation::Equal => Predicate::Eq(left, right),
                ProofRelation::NotEqual => Predicate::Ne(left, right),
                ProofRelation::Less => Predicate::Lt(left, right),
                ProofRelation::LessOrEqual => Predicate::Le(left, right),
                ProofRelation::Greater => Predicate::Gt(left, right),
                ProofRelation::GreaterOrEqual => Predicate::Ge(left, right),
            })
        }
        ProofProposition::InRange { value, start, end } => Some(Predicate::And(vec![
            Predicate::Ge(
                proof_term_to_normal(value, field_value, var_values)?,
                proof_term_to_normal(start, field_value, var_values)?,
            ),
            Predicate::Le(
                proof_term_to_normal(value, field_value, var_values)?,
                proof_term_to_normal(end, field_value, var_values)?,
            ),
        ])),
        ProofProposition::And(left, right) => Some(Predicate::And(vec![
            proposition_to_predicate(left, field_value, var_values)?,
            proposition_to_predicate(right, field_value, var_values)?,
        ])),
        ProofProposition::Not(_) | ProofProposition::Or(_, _) => None,
    }
}

fn proof_term_to_normal(
    term: &ast::ProofTerm,
    field_value: &NormalExpr,
    var_values: &HashMap<Intern<String>, NormalExpr>,
) -> Option<NormalExpr> {
    use ast::ProofTerm;
    let binary = |left: &ProofTerm, right: &ProofTerm| {
        Some((
            proof_term_to_normal(left, field_value, var_values)?,
            proof_term_to_normal(right, field_value, var_values)?,
        ))
    };
    match term {
        ProofTerm::Value(value) => Some(NormalExpr::Value(ast::ConstValue::Int(*value))),
        ProofTerm::Name(name) if name.as_str() == "self" => Some(field_value.clone()),
        ProofTerm::Name(name) => Some(
            var_values
                .get(name)
                .cloned()
                .unwrap_or(NormalExpr::Var(*name)),
        ),
        ProofTerm::Add(left, right) => {
            let (left, right) = binary(left, right)?;
            Some(NormalExpr::Add(Box::new(left), Box::new(right)))
        }
        ProofTerm::Sub(left, right) => {
            let (left, right) = binary(left, right)?;
            Some(NormalExpr::Sub(Box::new(left), Box::new(right)))
        }
        ProofTerm::Mul(left, right) => {
            let (left, right) = binary(left, right)?;
            Some(NormalExpr::Mul(Box::new(left), Box::new(right)))
        }
        ProofTerm::TargetQuery { kind, operand } => Some(NormalExpr::TargetQuery {
            kind: *kind,
            operand: operand.clone(),
        }),
        ProofTerm::Remainder(_, _) | ProofTerm::PowerOfTwo(_) => None,
    }
}
#[cfg(test)]
#[path = "../tests/solver_tests.rs"]
mod tests;
