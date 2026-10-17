use diagnostic::DiagnosticCode;
use diagnostic::code::{IntegerCode, TypeCode};
mod containment;

use std::num::NonZeroU32;

use i256::I256;

use crate::NormalExpr;

pub fn to_u128(value: I256) -> Option<u128> {
    (value >= I256::from(0) && value <= I256::from(u128::MAX)).then(|| value.as_u128())
}

pub fn to_i128(value: I256) -> Option<i128> {
    (value >= I256::from(i128::MIN) && value <= I256::from(i128::MAX)).then(|| value.as_i128())
}

pub fn to_u64(value: I256) -> Option<u64> {
    (value >= I256::from(0) && value <= I256::from(u64::MAX)).then(|| value.as_u64())
}

pub fn to_usize(value: I256) -> Option<usize> {
    to_u64(value).and_then(|value| usize::try_from(value).ok())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CanonicalIntegerExpr {
    Value(I256),
    Symbolic(NormalExpr),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FiniteHull {
    min: I256,
    max: I256,
}

impl FiniteHull {
    pub fn new(min: I256, max: I256) -> Option<Self> {
        (min <= max).then_some(Self { min, max })
    }

    pub fn min(&self) -> I256 {
        self.min
    }

    pub fn max(&self) -> I256 {
        self.max
    }

    pub fn contains(&self, value: I256) -> bool {
        self.min <= value && value <= self.max
    }

    pub fn inferred_representation(&self) -> IntegerRepresentation {
        let zero = I256::from(0);
        let width = if self.min >= zero {
            (I256::BITS - self.max.leading_zeros()).max(1)
        } else {
            self.signed_width()
        };
        IntegerRepresentation::new(NonZeroU32::new(width).unwrap())
    }

    fn signed_width(&self) -> u32 {
        for width in 1..I256::BITS {
            let magnitude = I256::from(1) << (width - 1);
            if self.min >= -magnitude && self.max < magnitude {
                return width;
            }
        }
        I256::BITS
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CanonicalPredicate {
    Empty,
    Unbounded,
    Symbolic(NormalExpr),
    Structured(crate::ty::PredicateExpr),
    LowerBound {
        bound: I256,
        inclusive: bool,
    },
    UpperBound {
        bound: I256,
        inclusive: bool,
    },
    Hull(FiniteHull),
    HullExcluding {
        hull: FiniteHull,
        excluded: Vec<I256>,
    },
    Union(Vec<CanonicalPredicate>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IntegerDomain {
    predicate: CanonicalPredicate,
    storage_hull: Option<FiniteHull>,
}

impl IntegerDomain {
    pub fn empty() -> Self {
        Self {
            predicate: CanonicalPredicate::Empty,
            storage_hull: None,
        }
    }

    pub fn unbounded() -> Self {
        Self {
            predicate: CanonicalPredicate::Unbounded,
            storage_hull: None,
        }
    }

    pub fn symbolic(predicate: NormalExpr) -> Self {
        Self {
            predicate: CanonicalPredicate::Symbolic(predicate),
            storage_hull: None,
        }
    }

    pub fn from_refinement(predicate: &crate::ty::PredicateExpr) -> Self {
        let predicate = predicate.flatten_conjunctions();
        let mut lower = None;
        let mut upper = None;
        let fully_represented_by_bounds = predicate.collect_bounds(&mut lower, &mut upper);
        if !fully_represented_by_bounds {
            let predicates = match &predicate {
                crate::ty::PredicateExpr::And(predicates) => predicates.as_slice(),
                _ => std::slice::from_ref(&predicate),
            };
            let mut excluded = Vec::new();
            let bounds_and_exclusions = predicates.iter().all(|predicate| {
                if let crate::ty::PredicateExpr::Ne(value) = predicate {
                    if let Some(value) = value.eval_integer() {
                        excluded.push(value);
                        return true;
                    }
                    return false;
                }
                predicate.collect_bounds(&mut lower, &mut upper)
            });
            if bounds_and_exclusions && let (Some(min), Some(max)) = (lower, upper) {
                return Self::excluding(min, max, excluded).unwrap_or_else(Self::empty);
            }
            let storage_hull = lower
                .zip(upper)
                .and_then(|(min, max)| FiniteHull::new(min, max));
            return Self {
                predicate: CanonicalPredicate::Structured(predicate),
                storage_hull,
            };
        }
        match (lower, upper) {
            (Some(min), Some(max)) => Self::bounded(min, max).unwrap_or_else(Self::empty),
            (Some(bound), None) => Self {
                predicate: CanonicalPredicate::LowerBound {
                    bound,
                    inclusive: true,
                },
                storage_hull: None,
            },
            (None, Some(bound)) => Self {
                predicate: CanonicalPredicate::UpperBound {
                    bound,
                    inclusive: true,
                },
                storage_hull: None,
            },
            (None, None) => Self::unbounded(),
        }
    }

    pub fn from_named_refinement(
        predicate: &crate::ty::PredicateExpr,
        subject: internment::Intern<String>,
    ) -> Self {
        let mut predicate = predicate.clone();
        predicate.normalize_subject(subject);
        Self::from_refinement(&predicate)
    }

    pub fn bounded(min: I256, max: I256) -> Option<Self> {
        let hull = FiniteHull::new(min, max)?;
        Some(Self {
            predicate: CanonicalPredicate::Hull(hull.clone()),
            storage_hull: Some(hull),
        })
    }

    pub fn excluding(min: I256, max: I256, excluded: Vec<I256>) -> Option<Self> {
        let hull = FiniteHull::new(min, max)?;
        let mut excluded: Vec<_> = excluded
            .into_iter()
            .filter(|value| hull.contains(*value))
            .collect();
        excluded.sort_unstable();
        excluded.dedup();
        let predicate = if excluded.is_empty() {
            CanonicalPredicate::Hull(hull.clone())
        } else {
            CanonicalPredicate::HullExcluding {
                hull: hull.clone(),
                excluded,
            }
        };
        Some(Self {
            predicate,
            storage_hull: Some(hull),
        })
    }

    pub fn union(domains: impl IntoIterator<Item = Self>) -> Self {
        let mut predicates = Vec::new();
        let mut min = None;
        let mut max = None;
        let mut all_finite = true;
        for domain in domains {
            match domain.predicate {
                CanonicalPredicate::Empty => continue,
                CanonicalPredicate::Unbounded => return Self::unbounded(),
                CanonicalPredicate::Union(nested) => predicates.extend(nested),
                predicate => predicates.push(predicate),
            }
            let Some(hull) = domain.storage_hull else {
                all_finite = false;
                continue;
            };
            min = Some(min.map_or(hull.min(), |current: I256| current.min(hull.min())));
            max = Some(max.map_or(hull.max(), |current: I256| current.max(hull.max())));
        }
        predicates.dedup();
        let storage_hull = all_finite
            .then(|| {
                min.zip(max)
                    .and_then(|(min, max)| FiniteHull::new(min, max))
            })
            .flatten();
        match predicates.len() {
            0 => Self::empty(),
            1 => Self {
                predicate: predicates.pop().expect("one predicate"),
                storage_hull,
            },
            _ => Self {
                predicate: CanonicalPredicate::Union(predicates),
                storage_hull,
            },
        }
    }

    pub fn predicate(&self) -> &CanonicalPredicate {
        &self.predicate
    }

    pub fn refine(&self, predicate: crate::ty::PredicateExpr) -> Self {
        use crate::ty::PredicateExpr;
        let mut predicates = match self.predicate() {
            CanonicalPredicate::Hull(hull) | CanonicalPredicate::HullExcluding { hull, .. } => {
                vec![
                    PredicateExpr::Ge(hull.min().into()),
                    PredicateExpr::Le(hull.max().into()),
                ]
            }
            CanonicalPredicate::Structured(predicate) => vec![predicate.clone()],
            _ => return self.clone(),
        };
        if let CanonicalPredicate::HullExcluding { excluded, .. } = self.predicate() {
            predicates.extend(
                excluded
                    .iter()
                    .map(|value| PredicateExpr::Ne((*value).into())),
            );
        }
        predicates.push(predicate);
        Self::from_refinement(&PredicateExpr::And(predicates))
    }

    pub fn storage_hull(&self) -> Option<&FiniteHull> {
        self.storage_hull.as_ref()
    }

    pub fn closed_bounds(&self) -> Option<(NormalExpr, NormalExpr)> {
        use crate::ty::PredicateExpr;
        match &self.predicate {
            CanonicalPredicate::Hull(hull) => Some((hull.min().into(), hull.max().into())),
            CanonicalPredicate::Structured(PredicateExpr::And(predicates)) => {
                let mut lower = None;
                let mut upper = None;
                for predicate in predicates {
                    match predicate {
                        PredicateExpr::Ge(bound) if lower.is_none() => lower = Some(bound.clone()),
                        PredicateExpr::Le(bound) if upper.is_none() => upper = Some(bound.clone()),
                        _ => return None,
                    }
                }
                lower.zip(upper)
            }
            _ => None,
        }
    }

    pub fn contains(&self, value: I256) -> bool {
        self.membership(value) == ContainmentProof::Proven
    }

    pub fn membership(&self, value: I256) -> ContainmentProof {
        let result = match &self.predicate {
            CanonicalPredicate::Empty => Some(false),
            CanonicalPredicate::Unbounded => Some(true),
            CanonicalPredicate::Symbolic(_) => None,
            CanonicalPredicate::Structured(predicate) => predicate.contains(value),
            CanonicalPredicate::LowerBound { bound, inclusive } => Some(if *inclusive {
                value >= *bound
            } else {
                value > *bound
            }),
            CanonicalPredicate::UpperBound { bound, inclusive } => Some(if *inclusive {
                value <= *bound
            } else {
                value < *bound
            }),
            CanonicalPredicate::Hull(hull) => Some(hull.contains(value)),
            CanonicalPredicate::HullExcluding { hull, excluded } => {
                Some(hull.contains(value) && excluded.binary_search(&value).is_err())
            }
            CanonicalPredicate::Union(predicates) => {
                let mut unknown = false;
                for predicate in predicates {
                    match (Self {
                        predicate: predicate.clone(),
                        storage_hull: None,
                    })
                    .membership(value)
                    {
                        ContainmentProof::Proven => return ContainmentProof::Proven,
                        ContainmentProof::Unknown => unknown = true,
                        ContainmentProof::Disproven => {}
                    }
                }
                if unknown { None } else { Some(false) }
            }
        };
        match result {
            Some(true) => ContainmentProof::Proven,
            Some(false) => ContainmentProof::Disproven,
            None => ContainmentProof::Unknown,
        }
    }
}

trait TruthValue {
    fn conjunction(self, other: Self) -> Self;
    fn disjunction(self, other: Self) -> Self;
}

impl TruthValue for Option<bool> {
    fn conjunction(self, other: Self) -> Self {
        match (self, other) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        }
    }

    fn disjunction(self, other: Self) -> Self {
        match (self, other) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        }
    }
}

impl NormalExpr {
    fn eval_integer(&self) -> Option<I256> {
        match self {
            Self::Value(crate::ConstValue::Int(value)) => Some(*value),
            Self::Add(left, right) => left.eval_integer()?.checked_add(right.eval_integer()?),
            Self::Sub(left, right) => left.eval_integer()?.checked_sub(right.eval_integer()?),
            Self::Mul(left, right) => left.eval_integer()?.checked_mul(right.eval_integer()?),
            Self::Value(_) | Self::Var(_) | Self::Inferred(_) | Self::TargetQuery { .. } => None,
        }
    }
}

impl crate::ty::ProofTerm {
    fn eval_integer(&self, candidate: I256) -> Option<I256> {
        match self {
            Self::Value(value) => Some(*value),
            Self::Name(name) if name.as_str() == "self" => Some(candidate),
            Self::Add(left, right) => Some(
                left.eval_integer(candidate)?
                    .wrapping_add(right.eval_integer(candidate)?),
            ),
            Self::Sub(left, right) => Some(
                left.eval_integer(candidate)?
                    .wrapping_sub(right.eval_integer(candidate)?),
            ),
            Self::Mul(left, right) => Some(
                left.eval_integer(candidate)?
                    .wrapping_mul(right.eval_integer(candidate)?),
            ),
            Self::Remainder(left, right) => {
                let left = left.eval_integer(candidate)?;
                let right = right.eval_integer(candidate)?;
                (right != I256::from(0)).then(|| left % right)
            }
            Self::PowerOfTwo(inner) => {
                let value = inner.eval_integer(candidate)?;
                let power =
                    value > I256::from(0) && (value & (value - I256::from(1))) == I256::from(0);
                Some(I256::from(u8::from(power)))
            }
            Self::Name(_) | Self::TargetQuery { .. } => None,
        }
    }

    fn normalize_subject(&mut self, subject: internment::Intern<String>) {
        match self {
            Self::Name(name) if *name == subject => {
                *name = internment::Intern::from_ref("self");
            }
            Self::Add(left, right)
            | Self::Sub(left, right)
            | Self::Mul(left, right)
            | Self::Remainder(left, right) => {
                left.normalize_subject(subject);
                right.normalize_subject(subject);
            }
            Self::PowerOfTwo(inner) => inner.normalize_subject(subject),
            Self::Name(_) | Self::Value(_) | Self::TargetQuery { .. } => {}
        }
    }
}

impl crate::ty::ProofProposition {
    fn holds(&self, candidate: I256) -> Option<bool> {
        use crate::ProofRelation;
        match self {
            Self::Compare {
                left,
                relation,
                right,
            } => {
                let left = left.eval_integer(candidate)?;
                let right = right.eval_integer(candidate)?;
                Some(match relation {
                    ProofRelation::Equal => left == right,
                    ProofRelation::NotEqual => left != right,
                    ProofRelation::Less => left < right,
                    ProofRelation::LessOrEqual => left <= right,
                    ProofRelation::Greater => left > right,
                    ProofRelation::GreaterOrEqual => left >= right,
                })
            }
            Self::InRange { value, start, end } => {
                let value = value.eval_integer(candidate)?;
                let start = start.eval_integer(candidate)?;
                let end = end.eval_integer(candidate)?;
                Some(start <= value && value <= end)
            }
            Self::Not(inner) => inner.holds(candidate).map(|value| !value),
            Self::And(left, right) => left.holds(candidate).conjunction(right.holds(candidate)),
            Self::Or(left, right) => left.holds(candidate).disjunction(right.holds(candidate)),
        }
    }

    fn normalize_subject(&mut self, subject: internment::Intern<String>) {
        match self {
            Self::Compare { left, right, .. } => {
                left.normalize_subject(subject);
                right.normalize_subject(subject);
            }
            Self::InRange { value, start, end } => {
                value.normalize_subject(subject);
                start.normalize_subject(subject);
                end.normalize_subject(subject);
            }
            Self::Not(inner) => inner.normalize_subject(subject),
            Self::And(left, right) | Self::Or(left, right) => {
                left.normalize_subject(subject);
                right.normalize_subject(subject);
            }
        }
    }
}

impl crate::ty::PredicateExpr {
    fn flatten_conjunctions(&self) -> Self {
        let Self::And(predicates) = self else {
            return self.clone();
        };
        let mut flattened = Vec::new();
        for predicate in predicates {
            match predicate.flatten_conjunctions() {
                Self::And(children) => flattened.extend(children),
                predicate => flattened.push(predicate),
            }
        }
        Self::And(flattened)
    }

    fn normalize_subject(&mut self, subject: internment::Intern<String>) {
        match self {
            Self::And(predicates) => {
                for predicate in predicates {
                    predicate.normalize_subject(subject);
                }
            }
            Self::Proposition(proposition) => proposition.normalize_subject(subject),
            Self::Lt(_) | Self::Gt(_) | Self::Le(_) | Self::Ge(_) | Self::Eq(_) | Self::Ne(_) => {}
        }
    }

    fn contains(&self, value: I256) -> Option<bool> {
        match self {
            Self::Lt(right) => right.eval_integer().map(|right| value < right),
            Self::Gt(right) => right.eval_integer().map(|right| value > right),
            Self::Le(right) => right.eval_integer().map(|right| value <= right),
            Self::Ge(right) => right.eval_integer().map(|right| value >= right),
            Self::Eq(right) => right.eval_integer().map(|right| value == right),
            Self::Ne(right) => right.eval_integer().map(|right| value != right),
            Self::And(predicates) => {
                let mut unknown = false;
                for predicate in predicates {
                    match predicate.contains(value) {
                        Some(false) => return Some(false),
                        None => unknown = true,
                        Some(true) => {}
                    }
                }
                (!unknown).then_some(true)
            }
            Self::Proposition(proposition) => proposition.holds(value),
        }
    }

    fn collect_bounds(&self, lower: &mut Option<I256>, upper: &mut Option<I256>) -> bool {
        match self {
            Self::Gt(expr) => expr
                .eval_integer()
                .and_then(|value| value.checked_add(I256::from(1)))
                .is_some_and(|value| {
                    *lower = Some(lower.map_or(value, |current| current.max(value)));
                    true
                }),
            Self::Ge(expr) => expr.eval_integer().is_some_and(|value| {
                *lower = Some(lower.map_or(value, |current| current.max(value)));
                true
            }),
            Self::Lt(expr) => expr
                .eval_integer()
                .and_then(|value| value.checked_sub(I256::from(1)))
                .is_some_and(|value| {
                    *upper = Some(upper.map_or(value, |current| current.min(value)));
                    true
                }),
            Self::Le(expr) => expr.eval_integer().is_some_and(|value| {
                *upper = Some(upper.map_or(value, |current| current.min(value)));
                true
            }),
            Self::Eq(expr) => expr.eval_integer().is_some_and(|value| {
                *lower = Some(lower.map_or(value, |current| current.max(value)));
                *upper = Some(upper.map_or(value, |current| current.min(value)));
                true
            }),
            Self::And(predicates) => predicates.iter().fold(true, |represented, predicate| {
                let child_represented = predicate.collect_bounds(lower, upper);
                represented && child_represented
            }),
            Self::Ne(_) | Self::Proposition(_) => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IntegerValidity(Box<IntegerDomain>);

impl IntegerValidity {
    pub fn new(domain: IntegerDomain) -> Self {
        Self(Box::new(domain))
    }

    pub fn domain(&self) -> &IntegerDomain {
        &self.0
    }

    pub fn resolve_representation(
        &self,
        rule: &IntegerRepresentationRule,
    ) -> Result<IntegerRepresentation, IntegerRepresentationError> {
        let required = self
            .domain()
            .storage_hull()
            .map(FiniteHull::inferred_representation)
            .ok_or_else(|| match self.domain().predicate() {
                CanonicalPredicate::Empty => IntegerRepresentationError::EmptyDomain,
                _ => IntegerRepresentationError::UnboundedDomain,
            })?;
        let representation = match rule {
            IntegerRepresentationRule::InferFromValidity => required,
            IntegerRepresentationRule::ExactBits(CanonicalNaturalExpr::Value(width)) => {
                IntegerRepresentation::new(*width)
            }
            IntegerRepresentationRule::ExactBits(CanonicalNaturalExpr::Symbolic(_)) => {
                return Err(IntegerRepresentationError::UnresolvedWidth);
            }
        };
        if representation.width() < required.width() {
            return Err(IntegerRepresentationError::RepresentationTooSmall {
                requested: representation,
                required,
            });
        }
        Ok(representation)
    }

    pub fn resolve_runtime_representation(
        &self,
        rule: &IntegerRepresentationRule,
    ) -> Result<IntegerRepresentation, IntegerRepresentationError> {
        let representation = self.resolve_representation(rule)?;
        if !representation.is_runtime_supported() {
            return Err(IntegerRepresentationError::RuntimeWidthUnsupported(
                representation,
            ));
        }
        Ok(representation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IntegerKnowledge {
    Exact(CanonicalIntegerExpr),
    Domain(IntegerDomain),
    Unknown,
    Poison,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainmentProof {
    Proven,
    Disproven,
    Unknown,
}

impl ContainmentProof {
    pub fn diagnostic_code(self) -> Option<DiagnosticCode> {
        match self {
            Self::Proven => None,
            Self::Disproven => Some(DiagnosticCode::Type(TypeCode::Integer(
                // type-integer-narrowing-failed
                IntegerCode::NarrowingFailed,
            ))),
            Self::Unknown => Some(DiagnosticCode::Type(TypeCode::Integer(
                // type-integer-narrowing-unproven
                IntegerCode::NarrowingUnproven,
            ))),
        }
    }
}

impl IntegerDomain {
    pub fn prove_containment(&self, expected: &IntegerDomain) -> ContainmentProof {
        containment::prove(self, expected)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CanonicalNaturalExpr {
    Value(NonZeroU32),
    Symbolic(NormalExpr),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IntegerRepresentationRule {
    InferFromValidity,
    ExactBits(CanonicalNaturalExpr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntegerRepresentation {
    width: NonZeroU32,
}

impl IntegerRepresentation {
    pub fn new(width: NonZeroU32) -> Self {
        Self { width }
    }

    pub fn width(self) -> NonZeroU32 {
        self.width
    }

    pub fn is_runtime_supported(self) -> bool {
        self.width.get() <= 128
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntegerInterpretation {
    Signed,
    Unsigned,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NominalIntegerSemantics {
    pub validity: IntegerValidity,
    pub representation_rule: IntegerRepresentationRule,
    pub interpretation: IntegerInterpretation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegerRepresentationError {
    EmptyDomain,
    UnboundedDomain,
    UnresolvedWidth,
    RuntimeWidthUnsupported(IntegerRepresentation),
    RepresentationTooSmall {
        requested: IntegerRepresentation,
        required: IntegerRepresentation,
    },
}

impl IntegerRepresentationError {
    pub fn diagnostic_code(&self) -> DiagnosticCode {
        match self {
            Self::EmptyDomain | Self::UnresolvedWidth => {
                // type-integer-representation-width-invalid
                DiagnosticCode::Type(TypeCode::Integer(IntegerCode::RepresentationWidthInvalid))
            }
            Self::UnboundedDomain => DiagnosticCode::Type(TypeCode::Integer(
                // type-integer-refinement-requires-finite-bounds
                IntegerCode::RefinementRequiresFiniteBounds,
            )),
            Self::RuntimeWidthUnsupported(_) => {
                // type-integer-runtime-width-unsupported
                DiagnosticCode::Type(TypeCode::Integer(IntegerCode::RuntimeWidthUnsupported))
            }
            Self::RepresentationTooSmall { .. } => {
                // type-integer-representation-too-small
                DiagnosticCode::Type(TypeCode::Integer(IntegerCode::RepresentationTooSmall))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::PredicateExpr;

    #[test]
    fn finite_refinements_canonicalize_exclusions_in_any_order() {
        let expected = IntegerDomain::excluding((-128).into(), 127.into(), vec![0.into()]).unwrap();
        for predicates in [
            vec![
                PredicateExpr::Ne(0_i128.into()),
                PredicateExpr::Ge((-128_i128).into()),
                PredicateExpr::Le(127_i128.into()),
            ],
            vec![
                PredicateExpr::Ge((-128_i128).into()),
                PredicateExpr::Le(127_i128.into()),
                PredicateExpr::Ne(0_i128.into()),
            ],
        ] {
            assert_eq!(
                IntegerDomain::from_refinement(&PredicateExpr::And(predicates)),
                expected
            );
        }
    }

    #[test]
    fn successive_guards_preserve_the_complete_intersection() {
        let expected = IntegerDomain::excluding((-128).into(), 127.into(), vec![0.into()]).unwrap();
        let actual = IntegerDomain::bounded((-32768).into(), 32767.into())
            .unwrap()
            .refine(PredicateExpr::And(vec![
                PredicateExpr::Ge((-128_i128).into()),
                PredicateExpr::Le(127_i128.into()),
            ]))
            .refine(PredicateExpr::Ne(0_i128.into()));
        assert_eq!(
            actual.prove_containment(&expected),
            ContainmentProof::Proven
        );
        assert!(!actual.contains(0.into()));
        let preserved = actual.refine(PredicateExpr::Le(100_i128.into()));
        assert!(!preserved.contains(0.into()));
        assert!(!preserved.contains(101.into()));
        assert!(preserved.contains(100.into()));
        assert_eq!(
            IntegerDomain::bounded((-128).into(), 127.into())
                .unwrap()
                .prove_containment(&expected),
            ContainmentProof::Disproven
        );
    }

    #[test]
    fn constant_arithmetic_bounds_establish_a_finite_signed_domain() {
        let negative = NormalExpr::Sub(Box::new(0_i128.into()), Box::new(128_i128.into()));
        let upper = NormalExpr::Sub(Box::new(128_i128.into()), Box::new(1_i128.into()));
        let domain = IntegerDomain::from_refinement(&PredicateExpr::And(vec![
            PredicateExpr::Ge(negative),
            PredicateExpr::Le(upper),
        ]));
        let hull = domain.storage_hull().expect("constant bounds");
        assert_eq!(hull.min(), I256::from(-128));
        assert_eq!(hull.max(), I256::from(127));
        assert!(domain.contains(I256::from(-3)));
        assert!(!domain.contains(I256::from(128)));
    }

    #[test]
    fn overflowing_bound_arithmetic_does_not_prove_a_wrapped_domain() {
        let overflow = NormalExpr::Add(Box::new(I256::MAX.into()), Box::new(1_i128.into()));
        assert_eq!(overflow.eval_integer(), None);
        let domain = IntegerDomain::from_refinement(&PredicateExpr::And(vec![
            PredicateExpr::Ge(overflow),
            PredicateExpr::Le(127_i128.into()),
        ]));
        assert!(domain.storage_hull().is_none());
    }

    #[test]
    fn symbolic_bounds_distinguish_unknown_membership_from_disproven_membership() {
        let predicates = vec![
            PredicateExpr::Ge(0_i128.into()),
            PredicateExpr::Le(NormalExpr::Var(internment::Intern::from_ref("maximum"))),
        ];
        for predicates in [predicates.clone(), predicates.into_iter().rev().collect()] {
            let domain = IntegerDomain::from_refinement(&PredicateExpr::And(predicates));
            assert_eq!(domain.membership((-1).into()), ContainmentProof::Disproven);
            assert_eq!(domain.membership(0.into()), ContainmentProof::Unknown);
        }
        let domain = IntegerDomain::from_refinement(&PredicateExpr::And(vec![
            PredicateExpr::Ge(0_i128.into()),
            PredicateExpr::Le(NormalExpr::Var(internment::Intern::from_ref("maximum"))),
        ]));
        let upper_bounded = IntegerDomain::from_refinement(&PredicateExpr::And(vec![
            PredicateExpr::Ge(NormalExpr::Var(internment::Intern::from_ref("minimum"))),
            PredicateExpr::Le(0_i128.into()),
        ]));
        assert_eq!(
            upper_bounded.membership(1.into()),
            ContainmentProof::Disproven
        );
        assert_eq!(
            upper_bounded.membership(0.into()),
            ContainmentProof::Unknown
        );
        let zero = IntegerDomain::bounded(0.into(), 0.into()).unwrap();
        assert_eq!(zero.prove_containment(&domain), ContainmentProof::Unknown);
        assert!(domain.closed_bounds().is_some());
    }

    #[test]
    fn range_projection_preserves_additional_domain_constraints() {
        let domain = IntegerDomain::from_refinement(&PredicateExpr::And(vec![
            PredicateExpr::Ge(0_i128.into()),
            PredicateExpr::Le(NormalExpr::Var(internment::Intern::from_ref("maximum"))),
            PredicateExpr::Ne(3_i128.into()),
        ]));
        assert!(domain.closed_bounds().is_none());
        assert_eq!(domain.membership(3.into()), ContainmentProof::Disproven);
        assert_eq!(domain.membership(2.into()), ContainmentProof::Unknown);
    }
}

#[cfg(test)]
#[path = "integer/containment_tests.rs"]
mod containment_tests;
