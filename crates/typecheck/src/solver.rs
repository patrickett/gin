use std::cell::OnceCell;
use std::collections::HashMap;

use ast::{ConstValue, NormalExpr, PredicateExpr};
use internment::Intern;

use crate::normal_expr::Normalize;
use crate::subst::DepSubst;

mod difference;
mod divisibility;
mod linear;

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
    Divisible(NormalExpr, NormalExpr),
    Opaque(ast::ProofProposition),
}

/// Known constraints on const variables, used to resolve predicate checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConstraintEnv {
    pub known_eq: Vec<(NormalExpr, NormalExpr)>,
    pub known_lt: Vec<(NormalExpr, NormalExpr)>,
    pub known_le: Vec<(NormalExpr, NormalExpr)>,
    pub known_divisible: Vec<(NormalExpr, NormalExpr)>,
}

/// A branch environment canonicalized once for a batch of proof queries.
///
/// It consumes the branch environment so that only one representation is
/// retained while candidates are compared, instead of holding the original
/// and canonical facts at the same time.
pub(crate) struct PreparedProofs<'a> {
    environment: ConstraintEnv,
    differences: OnceCell<difference::DifferenceGraph>,
    substitution: &'a DepSubst,
    aliases: DepSubst,
}

struct ProofQuery<'a> {
    environment: &'a ConstraintEnv,
    differences: &'a OnceCell<difference::DifferenceGraph>,
}

impl PreparedProofs<'_> {
    pub(crate) fn prove(&self, predicate: &Predicate) -> ProveResult {
        let query = ProofQuery {
            environment: &self.environment,
            differences: &self.differences,
        };
        if self.aliases.consts.is_empty() {
            return query.prove_predicate(predicate, self.substitution);
        }
        let canonical = |expression: &NormalExpr| {
            self.aliases
                .apply_to_normal(&self.substitution.apply_to_normal(expression))
                .normalize()
        };
        query.prove_predicate(&map_predicate(predicate, &canonical), &DepSubst::default())
    }
}

impl ConstraintEnv {
    pub fn forget(&mut self, name: Intern<String>) {
        fn mentions(expression: &NormalExpr, name: Intern<String>) -> bool {
            match expression {
                NormalExpr::Var(value) => {
                    *value == name || value.as_str().starts_with(&format!("{name}.$field"))
                }
                NormalExpr::Add(left, right)
                | NormalExpr::Sub(left, right)
                | NormalExpr::Mul(left, right) => mentions(left, name) || mentions(right, name),
                _ => false,
            }
        }
        self.known_eq
            .retain(|(left, right)| !mentions(left, name) && !mentions(right, name));
        self.known_lt
            .retain(|(left, right)| !mentions(left, name) && !mentions(right, name));
        self.known_le
            .retain(|(left, right)| !mentions(left, name) && !mentions(right, name));
        self.known_divisible
            .retain(|(left, right)| !mentions(left, name) && !mentions(right, name));
    }

    pub fn assume(&mut self, predicate: &Predicate) {
        match predicate {
            Predicate::Lt(left, right) => self.known_lt.push((left.normalize(), right.normalize())),
            Predicate::Gt(left, right) => self.known_lt.push((right.normalize(), left.normalize())),
            Predicate::Le(left, right) => self.known_le.push((left.normalize(), right.normalize())),
            Predicate::Ge(left, right) => self.known_le.push((right.normalize(), left.normalize())),
            Predicate::Eq(left, right) => {
                let left = left.normalize();
                let right = right.normalize();
                self.known_eq.push((left.clone(), right.clone()));
                self.known_le.push((left.clone(), right.clone()));
                self.known_le.push((right, left));
            }
            Predicate::Divisible(value, divisor) => self
                .known_divisible
                .push((value.normalize(), divisor.normalize())),
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
        let aliases = self.aliases(subst);
        if !aliases.consts.is_empty() {
            let canonical = |expression: &NormalExpr| {
                aliases
                    .apply_to_normal(&subst.apply_to_normal(expression))
                    .normalize()
            };
            let facts = |pairs: &[(NormalExpr, NormalExpr)]| {
                pairs
                    .iter()
                    .map(|(left, right)| (canonical(left), canonical(right)))
                    .collect()
            };
            let environment = Self {
                known_eq: facts(&self.known_eq),
                known_lt: facts(&self.known_lt),
                known_le: facts(&self.known_le),
                known_divisible: facts(&self.known_divisible),
            };
            let normalized = map_predicate(pred, &canonical);
            return environment.prove_predicate(&normalized, &DepSubst::default());
        }
        self.prove_predicate(pred, subst)
    }

    pub(crate) fn into_prepared_proofs<'a>(self, substitution: &'a DepSubst) -> PreparedProofs<'a> {
        let aliases = self.aliases(substitution);
        if aliases.consts.is_empty() {
            return PreparedProofs {
                environment: self,
                differences: OnceCell::new(),
                substitution,
                aliases,
            };
        }
        let canonical = |expression: &NormalExpr| {
            aliases
                .apply_to_normal(&substitution.apply_to_normal(expression))
                .normalize()
        };
        let facts = |pairs: &[(NormalExpr, NormalExpr)]| {
            pairs
                .iter()
                .map(|(left, right)| (canonical(left), canonical(right)))
                .collect()
        };
        PreparedProofs {
            environment: Self {
                known_eq: facts(&self.known_eq),
                known_lt: facts(&self.known_lt),
                known_le: facts(&self.known_le),
                known_divisible: facts(&self.known_divisible),
            },
            differences: OnceCell::new(),
            substitution,
            aliases,
        }
    }

    fn aliases(&self, subst: &DepSubst) -> DepSubst {
        fn root(
            parents: &HashMap<Intern<String>, Intern<String>>,
            mut name: Intern<String>,
        ) -> Intern<String> {
            while let Some(parent) = parents.get(&name) {
                if *parent == name {
                    break;
                }
                name = *parent;
            }
            name
        }
        let mut parents = HashMap::new();
        for (left, right) in &self.known_eq {
            if let (NormalExpr::Var(left), NormalExpr::Var(right)) =
                (subst.apply_to_normal(left), subst.apply_to_normal(right))
            {
                let left = root(&parents, left);
                let right = root(&parents, right);
                if left.as_str() < right.as_str() {
                    parents.insert(right, left);
                } else if right.as_str() < left.as_str() {
                    parents.insert(left, right);
                }
            }
        }
        let mut constants = HashMap::new();
        for (left, right) in &self.known_eq {
            match (subst.apply_to_normal(left), subst.apply_to_normal(right)) {
                (NormalExpr::Var(name), value @ NormalExpr::Value(ConstValue::Int(_)))
                | (value @ NormalExpr::Value(ConstValue::Int(_)), NormalExpr::Var(name)) => {
                    constants.insert(root(&parents, name), value);
                }
                _ => {}
            }
        }
        let mut consts: HashMap<_, _> = parents
            .keys()
            .map(|name| {
                let representative = root(&parents, *name);
                (
                    *name,
                    constants
                        .get(&representative)
                        .cloned()
                        .unwrap_or(NormalExpr::Var(representative)),
                )
            })
            .collect();
        consts.extend(constants);
        DepSubst {
            consts,
            ..Default::default()
        }
    }

    fn prove_predicate(&self, pred: &Predicate, subst: &DepSubst) -> ProveResult {
        ProofQuery {
            environment: self,
            differences: &OnceCell::new(),
        }
        .prove_predicate(pred, subst)
    }
}

impl ProofQuery<'_> {
    fn prove_predicate(&self, pred: &Predicate, subst: &DepSubst) -> ProveResult {
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
                    let r = self.prove_predicate(p, subst);
                    if r == ProveResult::Disproven {
                        return ProveResult::Disproven;
                    }
                    if r == ProveResult::Unknown {
                        result = ProveResult::Unknown;
                    }
                }
                result
            }
            Predicate::Divisible(value, divisor) => self.prove_divisible(value, divisor, subst),
            Predicate::Opaque(_) => ProveResult::Unknown,
        }
    }

    fn prove_lt(&self, l: &NormalExpr, r: &NormalExpr, subst: &DepSubst) -> ProveResult {
        self.prove_order(l, r, subst, true)
    }

    fn prove_order(
        &self,
        l: &NormalExpr,
        r: &NormalExpr,
        subst: &DepSubst,
        strict_required: bool,
    ) -> ProveResult {
        let l = subst.apply_to_normal(l).normalize();
        let r = subst.apply_to_normal(r).normalize();
        let integer_order = |left: &NormalExpr, right: &NormalExpr| match (left, right) {
            (
                NormalExpr::Value(ConstValue::Int(left)),
                NormalExpr::Value(ConstValue::Int(right)),
            ) => Some(left.cmp(right)),
            _ => None,
        };
        if let Some(order) = integer_order(&l, &r) {
            return if order.is_lt() || (!strict_required && order.is_eq()) {
                ProveResult::Proven
            } else {
                ProveResult::Disproven
            };
        }

        if self.prove_integer_difference(&l, &r, strict_required) == Some(ProveResult::Proven) {
            return ProveResult::Proven;
        }
        if self.prove_integer_difference(&r, &l, !strict_required) == Some(ProveResult::Proven) {
            return ProveResult::Disproven;
        }
        if self.environment.prove_linear_order(&l, &r, strict_required) {
            return ProveResult::Proven;
        }
        if self
            .environment
            .prove_linear_order(&r, &l, !strict_required)
        {
            return ProveResult::Disproven;
        }
        let arithmetic = self.addition_edges(&l, &r);
        let mut frontier = vec![(l, false)];
        let mut visited = Vec::new();
        while let Some((current, strict)) = frontier.pop() {
            if visited.contains(&(current.clone(), strict)) {
                continue;
            }
            visited.push((current.clone(), strict));
            let reaches_target = current == r && (strict || !strict_required)
                || integer_order(&current, &r).is_some_and(|order| {
                    order.is_lt() || (order.is_eq() && (strict || !strict_required))
                });
            if reaches_target {
                return ProveResult::Proven;
            }
            for (facts, edge_strict) in [
                (&self.environment.known_lt, true),
                (&self.environment.known_le, false),
            ] {
                for (left, right) in facts {
                    if *left == current {
                        frontier.push((right.clone(), strict || edge_strict));
                    } else if let Some(order) = integer_order(&current, left)
                        && order.is_le()
                    {
                        frontier.push((right.clone(), strict || edge_strict || order.is_lt()));
                    }
                }
            }
            for (left, right, edge_strict) in &arithmetic {
                if *left == current {
                    frontier.push((right.clone(), strict || *edge_strict));
                }
            }
        }
        ProveResult::Unknown
    }

    fn addition_edges(
        &self,
        left: &NormalExpr,
        right: &NormalExpr,
    ) -> Vec<(NormalExpr, NormalExpr, bool)> {
        let mut pending = vec![left, right];
        for (left, right) in self
            .environment
            .known_le
            .iter()
            .chain(&self.environment.known_lt)
        {
            pending.extend([left, right]);
        }
        let mut sums = std::collections::HashSet::new();
        while let Some(expression) = pending.pop() {
            if let NormalExpr::Add(left, right) = expression {
                if !sums.insert(expression.clone()) {
                    continue;
                }
                if sums.len() > 128 {
                    return Vec::new();
                }
                pending.extend([left.as_ref(), right.as_ref()]);
            }
        }
        let mut sums: Vec<_> = sums.into_iter().collect();
        sums.sort_by_key(|sum| format!("{sum:?}"));
        let mut edges = Vec::new();
        for sum in sums {
            let NormalExpr::Add(left, right) = &sum else {
                unreachable!();
            };
            for (base, offset) in [
                (left.as_ref(), right.as_ref()),
                (right.as_ref(), left.as_ref()),
            ] {
                if self.prove_integer_difference(&0_i128.into(), offset, false)
                    == Some(ProveResult::Proven)
                {
                    let strict = self.prove_integer_difference(&0_i128.into(), offset, true)
                        == Some(ProveResult::Proven);
                    edges.push((base.clone(), sum.clone(), strict));
                }
            }
        }
        edges
    }

    fn prove_integer_difference(
        &self,
        left: &NormalExpr,
        right: &NormalExpr,
        strict: bool,
    ) -> Option<ProveResult> {
        self.differences
            .get_or_init(|| difference::DifferenceGraph::new(self.environment))
            .prove(left, right, strict)
    }

    fn prove_eq(&self, l: &NormalExpr, r: &NormalExpr, subst: &DepSubst) -> ProveResult {
        let l = subst.apply_to_normal(l).normalize();
        let r = subst.apply_to_normal(r).normalize();

        // Check literal comparison
        match (&l, &r) {
            (NormalExpr::Value(a), NormalExpr::Value(b)) if a == b => return ProveResult::Proven,
            (NormalExpr::Value(_), NormalExpr::Value(_)) => return ProveResult::Disproven,
            _ if l == r => return ProveResult::Proven,
            _ => {}
        }

        if self
            .environment
            .known_eq
            .iter()
            .any(|(left, right)| (&l == left && &r == right) || (&l == right && &r == left))
            || (self.prove_integer_difference(&l, &r, false) == Some(ProveResult::Proven)
                && self.prove_integer_difference(&r, &l, false) == Some(ProveResult::Proven))
        {
            return ProveResult::Proven;
        }
        if self.prove_integer_difference(&l, &r, true) == Some(ProveResult::Proven)
            || self.prove_integer_difference(&r, &l, true) == Some(ProveResult::Proven)
        {
            return ProveResult::Disproven;
        }

        ProveResult::Unknown
    }

    fn prove_le(&self, l: &NormalExpr, r: &NormalExpr, subst: &DepSubst) -> ProveResult {
        let order = self.prove_order(l, r, subst, false);
        if order == ProveResult::Proven {
            return order;
        }
        order.or(self.prove_eq(l, r, subst))
    }
}

fn map_predicate(predicate: &Predicate, map: &impl Fn(&NormalExpr) -> NormalExpr) -> Predicate {
    match predicate {
        Predicate::Lt(left, right) => Predicate::Lt(map(left), map(right)),
        Predicate::Gt(left, right) => Predicate::Gt(map(left), map(right)),
        Predicate::Le(left, right) => Predicate::Le(map(left), map(right)),
        Predicate::Ge(left, right) => Predicate::Ge(map(left), map(right)),
        Predicate::Eq(left, right) => Predicate::Eq(map(left), map(right)),
        Predicate::Ne(left, right) => Predicate::Ne(map(left), map(right)),
        Predicate::Divisible(value, divisor) => Predicate::Divisible(map(value), map(divisor)),
        Predicate::And(predicates) => Predicate::And(
            predicates
                .iter()
                .map(|predicate| map_predicate(predicate, map))
                .collect(),
        ),
        Predicate::Opaque(proposition) => Predicate::Opaque(proposition.clone()),
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
    let substitution = DepSubst {
        types: HashMap::new(),
        consts: var_values.clone(),
    };
    let resolve = |rhs: &NormalExpr| substitution.apply_to_normal(rhs);
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
            if *relation == ProofRelation::Equal {
                for (remainder, zero) in [(left, right), (right, left)] {
                    if let ast::ProofTerm::Remainder(value, divisor) = remainder
                        && matches!(zero, ast::ProofTerm::Value(value) if *value == 0.into())
                    {
                        return Some(Predicate::Divisible(
                            proof_term_to_normal(value, field_value, var_values)?,
                            proof_term_to_normal(divisor, field_value, var_values)?,
                        ));
                    }
                }
            }
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

pub(crate) fn proof_term_to_normal(
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
        ProofTerm::Name(name) if name.as_str() == "self" => Some(
            var_values
                .get(name)
                .cloned()
                .unwrap_or_else(|| field_value.clone()),
        ),
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
mod prepared_proof_tests {
    use super::*;

    #[test]
    fn divisibility_survives_alias_canonicalization_and_prepared_proofs() {
        let address = NormalExpr::Var(Intern::from_ref("address"));
        let moved = NormalExpr::Var(Intern::from_ref("moved"));
        let mut environment = ConstraintEnv::default();
        environment.assume(&Predicate::Divisible(address.clone(), 4096_i128.into()));
        environment.assume(&Predicate::Eq(moved.clone(), address));
        let substitution = DepSubst::default();
        let prepared = environment.into_prepared_proofs(&substitution);
        assert_eq!(
            prepared.prove(&Predicate::Divisible(moved.clone(), 8_i128.into())),
            ProveResult::Proven
        );
        assert_eq!(
            prepared.prove(&Predicate::Divisible(moved.clone(), 3_i128.into())),
            ProveResult::Unknown
        );
        assert_eq!(
            prepared.prove(&Predicate::Divisible(moved, 0_i128.into())),
            ProveResult::Disproven
        );
    }

    #[test]
    fn prepared_alias_facts_apply_substitutions_and_keep_unknown_answers() {
        let a = Intern::from_ref("a");
        let b = Intern::from_ref("b");
        let limit = Intern::from_ref("limit");
        let mut environment = ConstraintEnv::default();
        environment.assume(&Predicate::Eq(NormalExpr::Var(a), NormalExpr::Var(b)));
        environment.assume(&Predicate::Eq(NormalExpr::Var(b), NormalExpr::Var(limit)));
        let mut substitution = DepSubst::default();
        substitution.consts.insert(limit, 7_i128.into());
        let prepared = environment.into_prepared_proofs(&substitution);
        for _ in 0..3 {
            for (predicate, expected) in [
                (
                    Predicate::Eq(NormalExpr::Var(a), 7_i128.into()),
                    ProveResult::Proven,
                ),
                (
                    Predicate::Lt(NormalExpr::Var(b), 7_i128.into()),
                    ProveResult::Disproven,
                ),
                (
                    Predicate::Le(NormalExpr::Var(a), NormalExpr::Var(b)),
                    ProveResult::Proven,
                ),
                (
                    Predicate::Eq(NormalExpr::Var(Intern::from_ref("missing")), 7_i128.into()),
                    ProveResult::Unknown,
                ),
            ] {
                assert_eq!(prepared.prove(&predicate), expected);
            }
        }
    }

    #[test]
    fn prepared_proofs_without_aliases_keep_substitutions_and_order_facts() {
        let x = Intern::from_ref("x");
        let bound = Intern::from_ref("bound");
        let mut environment = ConstraintEnv::default();
        environment.assume(&Predicate::Lt(NormalExpr::Var(x), 9_i128.into()));
        let mut substitution = DepSubst::default();
        substitution.consts.insert(bound, 9_i128.into());
        let prepared = environment.into_prepared_proofs(&substitution);
        for _ in 0..3 {
            assert_eq!(
                prepared.prove(&Predicate::Lt(NormalExpr::Var(x), 9_i128.into())),
                ProveResult::Proven
            );
            assert_eq!(
                prepared.prove(&Predicate::Eq(NormalExpr::Var(x), 9_i128.into())),
                ProveResult::Disproven
            );
            assert_eq!(
                prepared.prove(&Predicate::Eq(NormalExpr::Var(x), 4_i128.into())),
                ProveResult::Unknown
            );
        }
    }
}
