use super::*;
use internment::Intern;

#[test]
fn indexed_graph_matches_bounded_relaxation_with_cycles_offsets_and_overflow() {
    let x = NormalExpr::Var(Intern::from_ref("x"));
    let y = NormalExpr::Var(Intern::from_ref("y"));
    let z = NormalExpr::Var(Intern::from_ref("z"));
    let maximum = NormalExpr::Value(ConstValue::Int(I256::MAX));
    let minimum = NormalExpr::Value(ConstValue::Int(I256::MIN));
    let offset = NormalExpr::Add(Box::new(x.clone()), Box::new(maximum.clone()));
    let facts = [
        (x.clone(), y.clone(), false),
        (y.clone(), z.clone(), true),
        (z.clone(), x.clone(), false),
        (x.clone(), 0_i128.into(), true),
        (1_i128.into(), y.clone(), false),
        (maximum.clone(), z.clone(), false),
        (z.clone(), minimum.clone(), true),
        (offset.clone(), y.clone(), true),
    ];
    let expressions = [
        x.clone(),
        y.clone(),
        z,
        0_i128.into(),
        1_i128.into(),
        maximum,
        minimum,
        offset,
        NormalExpr::Sub(Box::new(y), Box::new(1_i128.into())),
        NormalExpr::Value(ConstValue::Proposition(false)),
    ];
    for mask in 0_u32..256 {
        let mut environment = ConstraintEnv::default();
        for (index, (left, right, strict)) in facts.iter().enumerate() {
            if mask & (1 << index) != 0 {
                let target = if *strict {
                    &mut environment.known_lt
                } else {
                    &mut environment.known_le
                };
                target.push((left.clone(), right.clone()));
            }
        }
        for reverse in [false, true] {
            if reverse {
                environment.known_le.reverse();
                environment.known_lt.reverse();
            }
            let graph = DifferenceGraph::new(&environment);
            for left in &expressions {
                for right in &expressions {
                    for strict in [false, true] {
                        assert_eq!(
                            graph.prove(left, right, strict),
                            reference_prove(&environment, left, right, strict),
                            "mask={mask} reverse={reverse} left={left:?} right={right:?} strict={strict}"
                        );
                    }
                }
            }
        }
    }
}

fn reference_prove(
    environment: &ConstraintEnv,
    left: &NormalExpr,
    right: &NormalExpr,
    strict: bool,
) -> Option<ProveResult> {
    let (left, left_offset) = split(left)?;
    let (right, right_offset) = split(right)?;
    let limit = right_offset
        .checked_sub(left_offset)?
        .checked_sub(I256::from(u8::from(strict)))?;
    let mut edges = Vec::new();
    for (facts, strict) in [
        (&environment.known_le, false),
        (&environment.known_lt, true),
    ] {
        for (left, right) in facts {
            if let (Some((left, left_offset)), Some((right, right_offset))) =
                (split(left), split(right))
                && let Some(weight) = right_offset
                    .checked_sub(left_offset)
                    .and_then(|weight| weight.checked_sub(I256::from(u8::from(strict))))
            {
                edges.push((left, right, weight));
            }
        }
    }
    let mut distances = HashMap::from([(left, I256::from(0))]);
    for _ in 0..=edges.len() {
        if distances
            .get(&right)
            .is_some_and(|distance| *distance <= limit)
        {
            return Some(ProveResult::Proven);
        }
        let mut changed = false;
        for (left, right, weight) in &edges {
            if let Some(next) = distances
                .get(left)
                .and_then(|distance| distance.checked_add(*weight))
                && distances.get(right).is_none_or(|distance| next < *distance)
            {
                distances.insert(right.clone(), next);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Some(ProveResult::Unknown)
}
