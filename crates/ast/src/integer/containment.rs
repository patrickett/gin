use super::{CanonicalPredicate, ContainmentProof, FiniteHull, IntegerDomain};
use crate::ty::PredicateExpr;
use i256::I256;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Endpoint {
    NegativeInfinity,
    BelowMinimum,
    Value(I256),
    AboveMaximum,
    PositiveInfinity,
}

impl Endpoint {
    fn before(value: I256) -> Self {
        value
            .checked_sub(I256::from(1))
            .map(Self::Value)
            .unwrap_or(Self::BelowMinimum)
    }

    fn after(value: I256) -> Self {
        value
            .checked_add(I256::from(1))
            .map(Self::Value)
            .unwrap_or(Self::AboveMaximum)
    }

    fn next(self) -> Option<Self> {
        match self {
            Self::BelowMinimum => Some(Self::Value(I256::MIN)),
            Self::Value(value) => Some(Self::after(value)),
            Self::NegativeInfinity | Self::AboveMaximum | Self::PositiveInfinity => None,
        }
    }
}

#[derive(Clone, Copy)]
struct Interval {
    min: Endpoint,
    max: Endpoint,
}

impl Interval {
    fn new(min: Endpoint, max: Endpoint) -> Option<Self> {
        (min <= max && min != Endpoint::PositiveInfinity && max != Endpoint::NegativeInfinity)
            .then_some(Self { min, max })
    }

    fn hull(hull: &FiniteHull) -> Self {
        Self {
            min: Endpoint::Value(hull.min()),
            max: Endpoint::Value(hull.max()),
        }
    }

    fn all() -> Self {
        Self {
            min: Endpoint::NegativeInfinity,
            max: Endpoint::PositiveInfinity,
        }
    }

    fn excluding(self, excluded: impl IntoIterator<Item = I256>) -> Vec<Self> {
        let mut excluded: Vec<_> = excluded
            .into_iter()
            .filter(|value| {
                self.min <= Endpoint::Value(*value) && Endpoint::Value(*value) <= self.max
            })
            .collect();
        excluded.sort_unstable();
        excluded.dedup();
        let mut result = Vec::new();
        let mut start = self.min;
        for value in excluded {
            result.extend(Self::new(start, Endpoint::before(value)));
            start = Endpoint::after(value);
        }
        result.extend(Self::new(start, self.max));
        result
    }
}

struct IntervalSet(Vec<Interval>);

impl IntervalSet {
    fn new(mut intervals: Vec<Interval>) -> Self {
        intervals.sort_unstable_by_key(|interval| (interval.min, interval.max));
        let mut merged: Vec<Interval> = Vec::new();
        for interval in intervals {
            if let Some(last) = merged.last_mut()
                && (interval.min <= last.max || last.max.next() == Some(interval.min))
            {
                last.max = last.max.max(interval.max);
            } else {
                merged.push(interval);
            }
        }
        Self(merged)
    }

    fn covers(&self, other: &Self) -> bool {
        let mut covering = self.0.iter().peekable();
        for wanted in &other.0 {
            while covering
                .peek()
                .is_some_and(|interval| interval.max < wanted.min)
            {
                covering.next();
            }
            let Some(interval) = covering.peek() else {
                return false;
            };
            if interval.min > wanted.min || interval.max < wanted.max {
                return false;
            }
        }
        true
    }
}

struct Approximation {
    possible: IntervalSet,
    definite: IntervalSet,
}

impl Approximation {
    fn new(domain: &IntegerDomain) -> Self {
        let mut possible = Vec::new();
        let mut definite = Vec::new();
        let mut pending = vec![domain.predicate()];
        while let Some(predicate) = pending.pop() {
            let exact = match predicate {
                CanonicalPredicate::Empty => Some(Vec::new()),
                CanonicalPredicate::Unbounded => Some(vec![Interval::all()]),
                CanonicalPredicate::Hull(hull) => Some(vec![Interval::hull(hull)]),
                CanonicalPredicate::HullExcluding { hull, excluded } => {
                    Some(Interval::hull(hull).excluding(excluded.iter().copied()))
                }
                CanonicalPredicate::LowerBound { bound, inclusive } => Some(
                    Interval::new(
                        if *inclusive {
                            Endpoint::Value(*bound)
                        } else {
                            Endpoint::after(*bound)
                        },
                        Endpoint::PositiveInfinity,
                    )
                    .into_iter()
                    .collect(),
                ),
                CanonicalPredicate::UpperBound { bound, inclusive } => Some(
                    Interval::new(
                        Endpoint::NegativeInfinity,
                        if *inclusive {
                            Endpoint::Value(*bound)
                        } else {
                            Endpoint::before(*bound)
                        },
                    )
                    .into_iter()
                    .collect(),
                ),
                CanonicalPredicate::Union(predicates) => {
                    pending.extend(predicates);
                    continue;
                }
                CanonicalPredicate::Symbolic(_) => {
                    possible.push(Interval::all());
                    continue;
                }
                CanonicalPredicate::Structured(predicate) => exact_refinement(predicate),
            };
            if let Some(exact) = exact {
                definite.extend(exact.iter().copied());
                possible.extend(exact);
            } else if let CanonicalPredicate::Structured(predicate) = predicate {
                let mut lower = None;
                let mut upper = None;
                predicate.collect_bounds(&mut lower, &mut upper);
                if let Some(interval) = Interval::new(
                    lower
                        .map(Endpoint::Value)
                        .unwrap_or(Endpoint::NegativeInfinity),
                    upper
                        .map(Endpoint::Value)
                        .unwrap_or(Endpoint::PositiveInfinity),
                ) {
                    let endpoints = [interval.min, interval.max];
                    let mut singleton_false = false;
                    for endpoint in endpoints {
                        if let Endpoint::Value(value) = endpoint {
                            match predicate.contains(value) {
                                Some(true) => definite.push(Interval {
                                    min: endpoint,
                                    max: endpoint,
                                }),
                                Some(false) if interval.min == interval.max => {
                                    singleton_false = true
                                }
                                Some(false) | None => {}
                            }
                        }
                    }
                    if !singleton_false {
                        possible.push(interval);
                    }
                }
            }
        }
        Self {
            possible: IntervalSet::new(possible),
            definite: IntervalSet::new(definite),
        }
    }
}

fn exact_refinement(predicate: &PredicateExpr) -> Option<Vec<Interval>> {
    let mut min = Endpoint::NegativeInfinity;
    let mut max = Endpoint::PositiveInfinity;
    let mut excluded = Vec::new();
    let mut pending = vec![predicate];
    while let Some(predicate) = pending.pop() {
        match predicate {
            PredicateExpr::Ge(value) => min = min.max(Endpoint::Value(value.eval_integer()?)),
            PredicateExpr::Gt(value) => min = min.max(Endpoint::after(value.eval_integer()?)),
            PredicateExpr::Le(value) => max = max.min(Endpoint::Value(value.eval_integer()?)),
            PredicateExpr::Lt(value) => max = max.min(Endpoint::before(value.eval_integer()?)),
            PredicateExpr::Eq(value) => {
                let value = Endpoint::Value(value.eval_integer()?);
                min = min.max(value);
                max = max.min(value);
            }
            PredicateExpr::Ne(value) => excluded.push(value.eval_integer()?),
            PredicateExpr::And(predicates) => pending.extend(predicates),
            PredicateExpr::Proposition(_) => return None,
        }
    }
    Some(Interval::new(min, max).map_or_else(Vec::new, |interval| interval.excluding(excluded)))
}

pub(super) fn prove(actual: &IntegerDomain, expected: &IntegerDomain) -> ContainmentProof {
    if actual == expected
        || matches!(actual.predicate(), CanonicalPredicate::Empty)
        || matches!(expected.predicate(), CanonicalPredicate::Unbounded)
    {
        return ContainmentProof::Proven;
    }
    if let CanonicalPredicate::Hull(expected_hull) = expected.predicate()
        && let Some(actual_hull) = actual.storage_hull()
        && expected_hull.contains(actual_hull.min())
        && expected_hull.contains(actual_hull.max())
    {
        return ContainmentProof::Proven;
    }
    if let Some(hull) = actual.storage_hull()
        && hull.min() == hull.max()
    {
        return match (
            actual.membership(hull.min()),
            expected.membership(hull.min()),
        ) {
            (ContainmentProof::Disproven, _) | (_, ContainmentProof::Proven) => {
                ContainmentProof::Proven
            }
            (ContainmentProof::Proven, ContainmentProof::Disproven) => ContainmentProof::Disproven,
            _ => ContainmentProof::Unknown,
        };
    }
    let actual = Approximation::new(actual);
    let expected = Approximation::new(expected);
    if expected.definite.covers(&actual.possible) {
        ContainmentProof::Proven
    } else if !expected.possible.covers(&actual.definite) {
        ContainmentProof::Disproven
    } else {
        ContainmentProof::Unknown
    }
}
