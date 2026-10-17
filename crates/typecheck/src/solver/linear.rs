use super::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, PartialEq, Eq)]
struct Bound {
    terms: HashMap<NormalExpr, i256::I256>,
    maximum: i256::I256,
}

impl Bound {
    fn new(left: &NormalExpr, right: &NormalExpr, strict: bool) -> Option<Self> {
        let mut terms = HashMap::new();
        let mut constant = i256::I256::from(u8::from(strict));
        let mut pending = vec![(left, i256::I256::from(1)), (right, i256::I256::from(-1))];
        let mut remaining = 128_usize;
        while let Some((expression, scale)) = pending.pop() {
            remaining = remaining.checked_sub(1)?;
            match expression {
                NormalExpr::Value(ConstValue::Int(value)) => {
                    constant = constant.checked_add(value.checked_mul(scale)?)?;
                }
                NormalExpr::Add(left, right) => {
                    pending.extend([(left.as_ref(), scale), (right.as_ref(), scale)])
                }
                NormalExpr::Sub(left, right) => pending.extend([
                    (left.as_ref(), scale),
                    (right.as_ref(), i256::I256::from(0).checked_sub(scale)?),
                ]),
                NormalExpr::Mul(left, right) => {
                    let (value, factor) = match (left.as_ref(), right.as_ref()) {
                        (NormalExpr::Value(ConstValue::Int(factor)), value)
                        | (value, NormalExpr::Value(ConstValue::Int(factor))) => (value, factor),
                        _ => {
                            let (atom, factor) = product_atom(expression)?;
                            let coefficient = terms.entry(atom).or_insert(i256::I256::from(0));
                            *coefficient = coefficient.checked_add(scale.checked_mul(factor)?)?;
                            continue;
                        }
                    };
                    pending.push((value, scale.checked_mul(*factor)?));
                }
                NormalExpr::Var(_) | NormalExpr::Inferred(_) | NormalExpr::TargetQuery { .. } => {
                    let coefficient = terms
                        .entry(expression.clone())
                        .or_insert(i256::I256::from(0));
                    *coefficient = coefficient.checked_add(scale)?;
                }
                NormalExpr::Value(_) => return None,
            }
        }
        terms.retain(|_, coefficient| *coefficient != 0.into());
        Some(Self {
            terms,
            maximum: i256::I256::from(0).checked_sub(constant)?,
        })
    }

    fn implies(&self, goal: &Self) -> Option<bool> {
        if self.terms.len() != goal.terms.len() {
            return Some(false);
        }
        let (name, coefficient) = self.terms.iter().next()?;
        let target = *goal.terms.get(name)?;
        if (*coefficient < 0.into()) != (target < 0.into()) {
            return Some(false);
        }
        let scale = coefficient.checked_abs()?;
        let target_scale = target.checked_abs()?;
        for (name, coefficient) in &self.terms {
            if coefficient.checked_mul(target_scale)? != goal.terms.get(name)?.checked_mul(scale)? {
                return Some(false);
            }
        }
        Some(self.maximum.checked_mul(target_scale)? <= goal.maximum.checked_mul(scale)?)
    }

    fn eliminate(&self, other: &Self, variable: &NormalExpr) -> Option<Self> {
        let left_scale = other.terms.get(variable)?.checked_abs()?;
        let right_scale = self.terms.get(variable)?.checked_abs()?;
        let mut terms = HashMap::new();
        for (source, scale) in [(self, left_scale), (other, right_scale)] {
            for (name, coefficient) in &source.terms {
                let value = terms.entry(name.clone()).or_insert(i256::I256::from(0));
                *value = value.checked_add(coefficient.checked_mul(scale)?)?;
            }
        }
        terms.retain(|_, coefficient| *coefficient != 0.into());
        Some(Self {
            terms,
            maximum: self
                .maximum
                .checked_mul(left_scale)?
                .checked_add(other.maximum.checked_mul(right_scale)?)?,
        })
    }
}

impl ConstraintEnv {
    pub(super) fn prove_linear_order(
        &self,
        left: &NormalExpr,
        right: &NormalExpr,
        strict: bool,
    ) -> bool {
        let Some(goal) = Bound::new(left, right, strict) else {
            return false;
        };
        if goal.terms.is_empty() {
            return goal.maximum >= 0.into();
        }
        for (facts, strict) in [(&self.known_le, false), (&self.known_lt, true)] {
            for (left, right) in facts {
                if Bound::new(left, right, strict).and_then(|known| known.implies(&goal))
                    == Some(true)
                {
                    return true;
                }
            }
        }
        let Some(negative) = Bound::new(right, left, !strict) else {
            return false;
        };
        self.linear_contradiction(negative).unwrap_or(false)
    }

    fn linear_contradiction(&self, negative: Bound) -> Option<bool> {
        let (mut variables, mut bounds) = connected_bounds(self, negative)?;
        for _ in 0..16 {
            if bounds
                .iter()
                .any(|bound| bound.terms.is_empty() && bound.maximum < 0.into())
            {
                return Some(true);
            }
            let variable = elimination_variable(&variables, &bounds)?;
            variables.remove(&variable);
            bounds = eliminate_variable(bounds, &variable)?;
            if variables.is_empty() {
                return Some(bounds.iter().any(|bound| bound.maximum < 0.into()));
            }
        }
        None
    }
}

fn product_atom(expression: &NormalExpr) -> Option<(NormalExpr, i256::I256)> {
    let mut pending = vec![expression];
    let mut factors = Vec::new();
    let mut scale = i256::I256::from(1);
    let mut remaining = 128_usize;
    while let Some(expression) = pending.pop() {
        remaining = remaining.checked_sub(1)?;
        match expression {
            NormalExpr::Mul(left, right) => pending.extend([left.as_ref(), right.as_ref()]),
            NormalExpr::Value(ConstValue::Int(value)) => scale = scale.checked_mul(*value)?,
            _ => factors.push(expression.clone()),
        }
    }
    factors.sort_by_key(|factor| format!("{factor:?}"));
    let mut factors = factors.into_iter();
    let atom = factors.next()?;
    Some((
        factors.fold(atom, |left, right| {
            NormalExpr::Mul(Box::new(left), Box::new(right))
        }),
        scale,
    ))
}

fn connected_bounds(
    environment: &ConstraintEnv,
    negative: Bound,
) -> Option<(HashSet<NormalExpr>, Vec<Bound>)> {
    let mut variables: HashSet<_> = negative.terms.keys().cloned().collect();
    let mut candidates: Vec<_> = [
        (&environment.known_le, false),
        (&environment.known_lt, true),
    ]
    .into_iter()
    .flat_map(|(facts, strict)| {
        facts
            .iter()
            .filter_map(move |(left, right)| Bound::new(left, right, strict))
    })
    .collect();
    let mut bounds = vec![negative];
    for _ in 0..16 {
        let mut connected = Vec::new();
        candidates.retain(|bound| {
            if bound.terms.keys().any(|name| variables.contains(name)) {
                connected.push(bound.clone());
                false
            } else {
                true
            }
        });
        if connected.is_empty() {
            break;
        }
        for bound in connected {
            variables.extend(bound.terms.keys().cloned());
            if !bounds.contains(&bound) {
                bounds.push(bound);
            }
        }
        if variables.len() > 16 || bounds.len() > 128 {
            return None;
        }
    }
    Some((variables, bounds))
}

fn elimination_variable(variables: &HashSet<NormalExpr>, bounds: &[Bound]) -> Option<NormalExpr> {
    variables
        .iter()
        .min_by_key(|variable| {
            let positive = bounds
                .iter()
                .filter(|bound| {
                    bound
                        .terms
                        .get(*variable)
                        .is_some_and(|coefficient| *coefficient > 0.into())
                })
                .count();
            let negative = bounds
                .iter()
                .filter(|bound| {
                    bound
                        .terms
                        .get(*variable)
                        .is_some_and(|coefficient| *coefficient < 0.into())
                })
                .count();
            (positive * negative, format!("{variable:?}"))
        })
        .cloned()
}

fn eliminate_variable(bounds: Vec<Bound>, variable: &NormalExpr) -> Option<Vec<Bound>> {
    let positive: Vec<_> = bounds
        .iter()
        .filter(|bound| {
            bound
                .terms
                .get(variable)
                .is_some_and(|coefficient| *coefficient > 0.into())
        })
        .collect();
    let negative: Vec<_> = bounds
        .iter()
        .filter(|bound| {
            bound
                .terms
                .get(variable)
                .is_some_and(|coefficient| *coefficient < 0.into())
        })
        .collect();
    let mut next = Vec::new();
    for left in positive {
        for right in &negative {
            let bound = left.eliminate(right, variable)?;
            if !next.contains(&bound) {
                next.push(bound);
            }
            if next.len() > 128 {
                return None;
            }
        }
    }
    for bound in bounds
        .into_iter()
        .filter(|bound| !bound.terms.contains_key(variable))
    {
        if !next.contains(&bound) {
            next.push(bound);
        }
        if next.len() > 128 {
            return None;
        }
    }
    Some(next)
}
