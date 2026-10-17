use super::{ConstValue, ConstraintEnv, NormalExpr, ProveResult};
use i256::I256;
use std::collections::HashMap;

pub(super) struct DifferenceGraph {
    nodes: HashMap<NormalExpr, usize>,
    edges: Vec<(usize, usize, I256)>,
}

impl DifferenceGraph {
    pub(super) fn new(environment: &ConstraintEnv) -> Self {
        let mut nodes = HashMap::new();
        let mut edges = Vec::new();
        for (facts, strict) in [
            (&environment.known_le, false),
            (&environment.known_lt, true),
        ] {
            for (left, right) in facts {
                let (Some((left, left_offset)), Some((right, right_offset))) =
                    (split(left), split(right))
                else {
                    continue;
                };
                let Some(weight) = right_offset
                    .checked_sub(left_offset)
                    .and_then(|weight| weight.checked_sub(I256::from(u8::from(strict))))
                else {
                    continue;
                };
                let next = nodes.len();
                let left = *nodes.entry(left).or_insert(next);
                let next = nodes.len();
                let right = *nodes.entry(right).or_insert(next);
                edges.push((left, right, weight));
            }
        }
        Self { nodes, edges }
    }

    pub(super) fn prove(
        &self,
        left: &NormalExpr,
        right: &NormalExpr,
        strict: bool,
    ) -> Option<ProveResult> {
        let (left, left_offset) = split(left)?;
        let (right, right_offset) = split(right)?;
        let limit = right_offset
            .checked_sub(left_offset)?
            .checked_sub(I256::from(u8::from(strict)))?;
        if left == right && I256::from(0) <= limit {
            return Some(ProveResult::Proven);
        }
        let (Some(&left), Some(&right)) = (self.nodes.get(&left), self.nodes.get(&right)) else {
            return Some(ProveResult::Unknown);
        };
        let mut distances = vec![None; self.nodes.len()];
        distances[left] = Some(I256::from(0));
        for _ in 0..=self.edges.len() {
            if distances[right].is_some_and(|distance| distance <= limit) {
                return Some(ProveResult::Proven);
            }
            let mut changed = false;
            for &(left, right, weight) in &self.edges {
                if let Some(next) =
                    distances[left].and_then(|distance| distance.checked_add(weight))
                    && distances[right].is_none_or(|distance| next < distance)
                {
                    distances[right] = Some(next);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Some(ProveResult::Unknown)
    }
}

fn split(value: &NormalExpr) -> Option<(NormalExpr, I256)> {
    match value {
        NormalExpr::Value(ConstValue::Int(value)) => Some((0_i128.into(), *value)),
        NormalExpr::Value(_) => None,
        NormalExpr::Add(left, right) => {
            let (left, offset) = split(left)?;
            if let NormalExpr::Value(ConstValue::Int(right)) = right.as_ref() {
                Some((left, offset.checked_add(*right)?))
            } else if let NormalExpr::Value(ConstValue::Int(constant)) = left {
                let (right, offset_right) = split(right)?;
                Some((
                    right,
                    constant.checked_add(offset)?.checked_add(offset_right)?,
                ))
            } else {
                Some((value.clone(), I256::from(0)))
            }
        }
        NormalExpr::Sub(left, right)
            if matches!(right.as_ref(), NormalExpr::Value(ConstValue::Int(_))) =>
        {
            let (left, offset) = split(left)?;
            let NormalExpr::Value(ConstValue::Int(right)) = right.as_ref() else {
                unreachable!()
            };
            Some((left, offset.checked_sub(*right)?))
        }
        _ => Some((value.clone(), I256::from(0))),
    }
}

#[cfg(test)]
mod tests;
