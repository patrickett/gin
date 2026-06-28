use std::collections::HashMap;

use ast::{ConstExpr, PredicateExpr};
use internment::Intern;

use crate::const_expr::Normalize;
use crate::subst::DepSubst;

/// A fully-resolved predicate with both sides explicit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    Lt(ConstExpr, ConstExpr),
    Gt(ConstExpr, ConstExpr),
    Le(ConstExpr, ConstExpr),
    Ge(ConstExpr, ConstExpr),
    Eq(ConstExpr, ConstExpr),
    Ne(ConstExpr, ConstExpr),
    And(Vec<Predicate>),
}

/// Known constraints on const variables, used to resolve predicate checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConstraintEnv {
    pub known_eq: Vec<(ConstExpr, ConstExpr)>,
    pub known_lt: Vec<(ConstExpr, ConstExpr)>,
    pub known_le: Vec<(ConstExpr, ConstExpr)>,
}

impl ConstraintEnv {
    /// Check whether a predicate holds given the current constraint environment.
    ///
    /// Variables in the predicate are looked up in `var_values` first — if a
    /// variable has a concrete value, it's substituted before checking.
    /// Falls through to `Unknown` when the info is insufficient.
    pub fn prove(
        &self,
        pred: &Predicate,
        var_values: &HashMap<Intern<String>, ConstExpr>,
    ) -> ProveResult {
        let subst = DepSubst {
            types: HashMap::new(),
            consts: var_values.clone(),
        };
        match pred {
            Predicate::Lt(l, r) => self.prove_lt(l, r, &subst),
            Predicate::Gt(l, r) => self.prove_lt(r, l, &subst),
            Predicate::Le(l, r) => self.prove_lt(l, r, &subst).or(self.prove_eq(l, r, &subst)),
            Predicate::Ge(l, r) => self.prove_lt(r, l, &subst).or(self.prove_eq(l, r, &subst)),
            Predicate::Eq(l, r) => self.prove_eq(l, r, &subst),
            Predicate::Ne(l, r) => match self.prove_eq(l, r, &subst) {
                ProveResult::Proven => ProveResult::Disproven,
                ProveResult::Disproven => ProveResult::Proven,
                ProveResult::Unknown => ProveResult::Unknown,
            },
            Predicate::And(preds) => {
                let mut result = ProveResult::Proven;
                for p in preds {
                    let r = self.prove(p, var_values);
                    if r == ProveResult::Disproven {
                        return ProveResult::Disproven;
                    }
                    if r == ProveResult::Unknown {
                        result = ProveResult::Unknown;
                    }
                }
                result
            }
        }
    }

    fn prove_lt(&self, l: &ConstExpr, r: &ConstExpr, subst: &DepSubst) -> ProveResult {
        let l = subst.apply_to_const(l).normalize();
        let r = subst.apply_to_const(r).normalize();

        // Check literal comparison
        if let (ConstExpr::Value(a), ConstExpr::Value(b)) = (&l, &r) {
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

        ProveResult::Unknown
    }

    fn prove_eq(&self, l: &ConstExpr, r: &ConstExpr, subst: &DepSubst) -> ProveResult {
        let l = subst.apply_to_const(l).normalize();
        let r = subst.apply_to_const(r).normalize();

        // Check literal comparison
        match (&l, &r) {
            (ConstExpr::Value(a), ConstExpr::Value(b)) if a == b => return ProveResult::Proven,
            (ConstExpr::Value(_), ConstExpr::Value(_)) => return ProveResult::Disproven,
            _ if l == r => return ProveResult::Proven,
            _ => {}
        }

        // Check known_eq facts in the constraint env
        if self.known_eq.iter().any(|(kl, kr)| &l == kl && &r == kr) {
            return ProveResult::Proven;
        }

        ProveResult::Unknown
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
    field_value: ConstExpr,
    var_values: &HashMap<Intern<String>, ConstExpr>,
) -> Predicate {
    let resolve = |rhs: &ConstExpr| -> ConstExpr {
        match rhs {
            ConstExpr::Var(name) => var_values.get(name).cloned().unwrap_or(rhs.clone()),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::{ConstExpr, ConstValue};
    use internment::Intern;

    fn env() -> ConstraintEnv {
        ConstraintEnv::default()
    }

    fn vals() -> HashMap<Intern<String>, ConstExpr> {
        HashMap::new()
    }

    #[test]
    fn literal_lt_true() {
        let p = Predicate::Lt(
            ConstExpr::Value(ConstValue::Int(3)),
            ConstExpr::Value(ConstValue::Int(5)),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Proven);
    }

    #[test]
    fn literal_lt_false() {
        let p = Predicate::Lt(
            ConstExpr::Value(ConstValue::Int(5)),
            ConstExpr::Value(ConstValue::Int(3)),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Disproven);
    }

    #[test]
    fn literal_eq_true() {
        let p = Predicate::Eq(
            ConstExpr::Value(ConstValue::Int(42)),
            ConstExpr::Value(ConstValue::Int(42)),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Proven);
    }

    #[test]
    fn literal_eq_false() {
        let p = Predicate::Eq(
            ConstExpr::Value(ConstValue::Int(1)),
            ConstExpr::Value(ConstValue::Int(2)),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Disproven);
    }

    #[test]
    fn var_unknown() {
        let p = Predicate::Lt(
            ConstExpr::Var(Intern::from_ref("n")),
            ConstExpr::Var(Intern::from_ref("m")),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Unknown);
    }

    #[test]
    fn var_resolved_from_map() {
        let mut vals = HashMap::new();
        vals.insert(Intern::from_ref("n"), ConstExpr::Value(ConstValue::Int(3)));
        vals.insert(Intern::from_ref("m"), ConstExpr::Value(ConstValue::Int(5)));
        let p = Predicate::Lt(
            ConstExpr::Var(Intern::from_ref("n")),
            ConstExpr::Var(Intern::from_ref("m")),
        );
        assert_eq!(env().prove(&p, &vals), ProveResult::Proven);
    }

    #[test]
    fn and_both_true() {
        let p = Predicate::And(vec![
            Predicate::Lt(
                ConstExpr::Value(ConstValue::Int(1)),
                ConstExpr::Value(ConstValue::Int(2)),
            ),
            Predicate::Lt(
                ConstExpr::Value(ConstValue::Int(2)),
                ConstExpr::Value(ConstValue::Int(3)),
            ),
        ]);
        assert_eq!(env().prove(&p, &vals()), ProveResult::Proven);
    }

    #[test]
    fn and_one_false() {
        let p = Predicate::And(vec![
            Predicate::Lt(
                ConstExpr::Value(ConstValue::Int(1)),
                ConstExpr::Value(ConstValue::Int(2)),
            ),
            Predicate::Lt(
                ConstExpr::Value(ConstValue::Int(5)),
                ConstExpr::Value(ConstValue::Int(3)),
            ),
        ]);
        assert_eq!(env().prove(&p, &vals()), ProveResult::Disproven);
    }

    #[test]
    fn and_mixed_unknown() {
        let p = Predicate::And(vec![
            Predicate::Lt(
                ConstExpr::Value(ConstValue::Int(1)),
                ConstExpr::Value(ConstValue::Int(2)),
            ),
            Predicate::Lt(
                ConstExpr::Var(Intern::from_ref("n")),
                ConstExpr::Var(Intern::from_ref("m")),
            ),
        ]);
        assert_eq!(env().prove(&p, &vals()), ProveResult::Unknown);
    }

    #[test]
    fn normalized_identity() {
        let p = Predicate::Eq(
            ConstExpr::Add(
                Box::new(ConstExpr::Var(Intern::from_ref("n"))),
                Box::new(ConstExpr::Value(ConstValue::Int(0))),
            ),
            ConstExpr::Var(Intern::from_ref("n")),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Proven);
    }

    #[test]
    fn ne_true() {
        let p = Predicate::Ne(
            ConstExpr::Value(ConstValue::Int(1)),
            ConstExpr::Value(ConstValue::Int(2)),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Proven);
    }

    #[test]
    fn ne_false() {
        let p = Predicate::Ne(
            ConstExpr::Value(ConstValue::Int(42)),
            ConstExpr::Value(ConstValue::Int(42)),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Disproven);
    }

    #[test]
    fn le_true() {
        let p = Predicate::Le(
            ConstExpr::Value(ConstValue::Int(3)),
            ConstExpr::Value(ConstValue::Int(5)),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Proven);
    }

    #[test]
    fn le_equal_true() {
        let p = Predicate::Le(
            ConstExpr::Value(ConstValue::Int(5)),
            ConstExpr::Value(ConstValue::Int(5)),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Proven);
    }

    #[test]
    fn le_false() {
        let p = Predicate::Le(
            ConstExpr::Value(ConstValue::Int(6)),
            ConstExpr::Value(ConstValue::Int(3)),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Disproven);
    }

    #[test]
    fn ge_true() {
        let p = Predicate::Ge(
            ConstExpr::Value(ConstValue::Int(5)),
            ConstExpr::Value(ConstValue::Int(3)),
        );
        assert_eq!(env().prove(&p, &vals()), ProveResult::Proven);
    }
}
