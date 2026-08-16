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
            signed_width(self.min, self.max)
        };
        IntegerRepresentation::new(NonZeroU32::new(width).unwrap())
    }
}

fn signed_width(min: I256, max: I256) -> u32 {
    for width in 1..I256::BITS {
        let magnitude = I256::from(1) << (width - 1);
        if min >= -magnitude && max < magnitude {
            return width;
        }
    }
    I256::BITS
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
        let predicate = flatten_refinement_conjunctions(predicate);
        let mut lower = None;
        let mut upper = None;
        let fully_represented_by_bounds =
            collect_refinement_bounds(&predicate, &mut lower, &mut upper);
        if !fully_represented_by_bounds {
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
        normalize_refinement_subject(&mut predicate, subject);
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

    pub fn storage_hull(&self) -> Option<&FiniteHull> {
        self.storage_hull.as_ref()
    }

    pub fn contains(&self, value: I256) -> bool {
        match &self.predicate {
            CanonicalPredicate::Empty => false,
            CanonicalPredicate::Unbounded => true,
            CanonicalPredicate::Symbolic(_) => false,
            CanonicalPredicate::Structured(predicate) => {
                structured_predicate_contains(predicate, value)
            }
            CanonicalPredicate::LowerBound { bound, inclusive } => {
                if *inclusive {
                    value >= *bound
                } else {
                    value > *bound
                }
            }
            CanonicalPredicate::UpperBound { bound, inclusive } => {
                if *inclusive {
                    value <= *bound
                } else {
                    value < *bound
                }
            }
            CanonicalPredicate::Hull(hull) => hull.contains(value),
            CanonicalPredicate::HullExcluding { hull, excluded } => {
                hull.contains(value) && excluded.binary_search(&value).is_err()
            }
            CanonicalPredicate::Union(predicates) => predicates.iter().any(|predicate| {
                Self {
                    predicate: predicate.clone(),
                    storage_hull: self.storage_hull.clone(),
                }
                .contains(value)
            }),
        }
    }
}

fn flatten_refinement_conjunctions(
    predicate: &crate::ty::PredicateExpr,
) -> crate::ty::PredicateExpr {
    let crate::ty::PredicateExpr::And(predicates) = predicate else {
        return predicate.clone();
    };
    let mut flattened = Vec::new();
    for predicate in predicates {
        match flatten_refinement_conjunctions(predicate) {
            crate::ty::PredicateExpr::And(children) => flattened.extend(children),
            predicate => flattened.push(predicate),
        }
    }
    crate::ty::PredicateExpr::And(flattened)
}

fn normalize_refinement_subject(
    predicate: &mut crate::ty::PredicateExpr,
    subject: internment::Intern<String>,
) {
    use crate::ty::PredicateExpr;
    match predicate {
        PredicateExpr::And(predicates) => {
            for predicate in predicates {
                normalize_refinement_subject(predicate, subject);
            }
        }
        PredicateExpr::Proposition(proposition) => {
            normalize_proposition_subject(proposition, subject);
        }
        PredicateExpr::Lt(_)
        | PredicateExpr::Gt(_)
        | PredicateExpr::Le(_)
        | PredicateExpr::Ge(_)
        | PredicateExpr::Eq(_)
        | PredicateExpr::Ne(_) => {}
    }
}

fn normalize_proposition_subject(
    proposition: &mut crate::ProofProposition,
    subject: internment::Intern<String>,
) {
    use crate::ProofProposition;
    match proposition {
        ProofProposition::Compare { left, right, .. } => {
            normalize_proof_term_subject(left, subject);
            normalize_proof_term_subject(right, subject);
        }
        ProofProposition::InRange { value, start, end } => {
            normalize_proof_term_subject(value, subject);
            normalize_proof_term_subject(start, subject);
            normalize_proof_term_subject(end, subject);
        }
        ProofProposition::Not(inner) => normalize_proposition_subject(inner, subject),
        ProofProposition::And(left, right) | ProofProposition::Or(left, right) => {
            normalize_proposition_subject(left, subject);
            normalize_proposition_subject(right, subject);
        }
    }
}

fn normalize_proof_term_subject(term: &mut crate::ProofTerm, subject: internment::Intern<String>) {
    use crate::ProofTerm;
    match term {
        ProofTerm::Name(name) if *name == subject => {
            *name = internment::Intern::from_ref("self");
        }
        ProofTerm::Add(left, right)
        | ProofTerm::Sub(left, right)
        | ProofTerm::Mul(left, right)
        | ProofTerm::Remainder(left, right) => {
            normalize_proof_term_subject(left, subject);
            normalize_proof_term_subject(right, subject);
        }
        ProofTerm::PowerOfTwo(inner) => normalize_proof_term_subject(inner, subject),
        ProofTerm::Name(_) | ProofTerm::Value(_) | ProofTerm::TargetQuery { .. } => {}
    }
}

fn structured_predicate_contains(predicate: &crate::ty::PredicateExpr, value: I256) -> bool {
    use crate::ty::PredicateExpr;
    match predicate {
        PredicateExpr::Lt(right) => eval_normal_expr(right).is_some_and(|right| value < right),
        PredicateExpr::Gt(right) => eval_normal_expr(right).is_some_and(|right| value > right),
        PredicateExpr::Le(right) => eval_normal_expr(right).is_some_and(|right| value <= right),
        PredicateExpr::Ge(right) => eval_normal_expr(right).is_some_and(|right| value >= right),
        PredicateExpr::Eq(right) => eval_normal_expr(right).is_some_and(|right| value == right),
        PredicateExpr::Ne(right) => eval_normal_expr(right).is_some_and(|right| value != right),
        PredicateExpr::And(predicates) => predicates
            .iter()
            .all(|predicate| structured_predicate_contains(predicate, value)),
        PredicateExpr::Proposition(proposition) => eval_proposition(proposition, value),
    }
}

fn eval_normal_expr(expr: &NormalExpr) -> Option<I256> {
    match expr {
        NormalExpr::Value(crate::ConstValue::Int(value)) => Some(*value),
        NormalExpr::Add(left, right) => {
            Some(eval_normal_expr(left)?.wrapping_add(eval_normal_expr(right)?))
        }
        NormalExpr::Sub(left, right) => {
            Some(eval_normal_expr(left)?.wrapping_sub(eval_normal_expr(right)?))
        }
        NormalExpr::Mul(left, right) => {
            Some(eval_normal_expr(left)?.wrapping_mul(eval_normal_expr(right)?))
        }
        NormalExpr::Value(_)
        | NormalExpr::Var(_)
        | NormalExpr::Inferred(_)
        | NormalExpr::TargetQuery { .. } => None,
    }
}

fn eval_proposition(proposition: &crate::ProofProposition, candidate: I256) -> bool {
    use crate::{ProofProposition, ProofRelation};
    match proposition {
        ProofProposition::Compare {
            left,
            relation,
            right,
        } => {
            let Some(left) = eval_proof_term(left, candidate) else {
                return false;
            };
            let Some(right) = eval_proof_term(right, candidate) else {
                return false;
            };
            match relation {
                ProofRelation::Equal => left == right,
                ProofRelation::NotEqual => left != right,
                ProofRelation::Less => left < right,
                ProofRelation::LessOrEqual => left <= right,
                ProofRelation::Greater => left > right,
                ProofRelation::GreaterOrEqual => left >= right,
            }
        }
        ProofProposition::InRange { value, start, end } => {
            let Some(value) = eval_proof_term(value, candidate) else {
                return false;
            };
            let Some(start) = eval_proof_term(start, candidate) else {
                return false;
            };
            let Some(end) = eval_proof_term(end, candidate) else {
                return false;
            };
            start <= value && value <= end
        }
        ProofProposition::Not(inner) => !eval_proposition(inner, candidate),
        ProofProposition::And(left, right) => {
            eval_proposition(left, candidate) && eval_proposition(right, candidate)
        }
        ProofProposition::Or(left, right) => {
            eval_proposition(left, candidate) || eval_proposition(right, candidate)
        }
    }
}

fn eval_proof_term(term: &crate::ProofTerm, candidate: I256) -> Option<I256> {
    use crate::ProofTerm;
    match term {
        ProofTerm::Value(value) => Some(*value),
        ProofTerm::Name(name) if name.as_str() == "self" => Some(candidate),
        ProofTerm::Add(left, right) => {
            Some(eval_proof_term(left, candidate)?.wrapping_add(eval_proof_term(right, candidate)?))
        }
        ProofTerm::Sub(left, right) => {
            Some(eval_proof_term(left, candidate)?.wrapping_sub(eval_proof_term(right, candidate)?))
        }
        ProofTerm::Mul(left, right) => {
            Some(eval_proof_term(left, candidate)?.wrapping_mul(eval_proof_term(right, candidate)?))
        }
        ProofTerm::Remainder(left, right) => {
            let left = eval_proof_term(left, candidate)?;
            let right = eval_proof_term(right, candidate)?;
            (right != I256::from(0)).then(|| left % right)
        }
        ProofTerm::PowerOfTwo(inner) => {
            let value = eval_proof_term(inner, candidate)?;
            let power = value > I256::from(0) && (value & (value - I256::from(1))) == I256::from(0);
            Some(I256::from(u8::from(power)))
        }
        ProofTerm::Name(_) | ProofTerm::TargetQuery { .. } => None,
    }
}

fn collect_refinement_bounds(
    predicate: &crate::ty::PredicateExpr,
    lower: &mut Option<I256>,
    upper: &mut Option<I256>,
) -> bool {
    use crate::ty::PredicateExpr;

    let value = |expr: &NormalExpr| match expr {
        NormalExpr::Value(crate::ConstValue::Int(value)) => Some(*value),
        _ => None,
    };
    match predicate {
        PredicateExpr::Gt(expr) => value(expr)
            .and_then(|value| value.checked_add(I256::from(1)))
            .is_some_and(|value| {
                *lower = Some(lower.map_or(value, |current| current.max(value)));
                true
            }),
        PredicateExpr::Ge(expr) => value(expr).is_some_and(|value| {
            *lower = Some(lower.map_or(value, |current| current.max(value)));
            true
        }),
        PredicateExpr::Lt(expr) => value(expr)
            .and_then(|value| value.checked_sub(I256::from(1)))
            .is_some_and(|value| {
                *upper = Some(upper.map_or(value, |current| current.min(value)));
                true
            }),
        PredicateExpr::Le(expr) => value(expr).is_some_and(|value| {
            *upper = Some(upper.map_or(value, |current| current.min(value)));
            true
        }),
        PredicateExpr::Eq(expr) => value(expr).is_some_and(|value| {
            *lower = Some(lower.map_or(value, |current| current.max(value)));
            *upper = Some(upper.map_or(value, |current| current.min(value)));
            true
        }),
        PredicateExpr::And(predicates) => predicates
            .iter()
            .all(|predicate| collect_refinement_bounds(predicate, lower, upper)),
        PredicateExpr::Ne(_) | PredicateExpr::Proposition(_) => false,
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
    pub fn diagnostic_code(self) -> Option<&'static str> {
        match self {
            Self::Proven => None,
            Self::Disproven => Some("type-integer-narrowing-failed"),
            Self::Unknown => Some("type-integer-narrowing-unproven"),
        }
    }
}

pub fn prove_containment(actual: &IntegerDomain, expected: &IntegerDomain) -> ContainmentProof {
    if actual == expected {
        return ContainmentProof::Proven;
    }
    if matches!(actual.predicate(), CanonicalPredicate::Empty)
        || matches!(expected.predicate(), CanonicalPredicate::Unbounded)
    {
        return ContainmentProof::Proven;
    }
    if let Some(actual_hull) = actual.storage_hull()
        && actual_hull.min() == actual_hull.max()
    {
        return if expected.contains(actual_hull.min()) {
            ContainmentProof::Proven
        } else {
            ContainmentProof::Disproven
        };
    }
    if matches!(
        actual.predicate(),
        CanonicalPredicate::Symbolic(_) | CanonicalPredicate::Structured(_)
    ) || matches!(
        expected.predicate(),
        CanonicalPredicate::Symbolic(_) | CanonicalPredicate::Structured(_)
    ) {
        return ContainmentProof::Unknown;
    }
    if matches!(actual.predicate(), CanonicalPredicate::Unbounded) {
        return ContainmentProof::Disproven;
    }
    let (Some(actual_hull), Some(expected_hull)) = (actual.storage_hull(), expected.storage_hull())
    else {
        return ContainmentProof::Disproven;
    };
    if actual_hull.min() < expected_hull.min() || actual_hull.max() > expected_hull.max() {
        return ContainmentProof::Disproven;
    }
    if let CanonicalPredicate::HullExcluding { excluded, .. } = expected.predicate()
        && excluded.iter().any(|value| actual.contains(*value))
    {
        return ContainmentProof::Disproven;
    }
    ContainmentProof::Proven
}

pub fn proves_lossless_storage(
    knowledge: &IntegerKnowledge,
    target: IntegerRepresentation,
) -> bool {
    let required = match knowledge {
        IntegerKnowledge::Exact(CanonicalIntegerExpr::Value(value)) => {
            FiniteHull::new(*value, *value).map(|hull| hull.inferred_representation())
        }
        IntegerKnowledge::Domain(domain) => domain
            .storage_hull()
            .map(FiniteHull::inferred_representation),
        IntegerKnowledge::Exact(CanonicalIntegerExpr::Symbolic(_))
        | IntegerKnowledge::Unknown
        | IntegerKnowledge::Poison => None,
    };
    required.is_some_and(|required| required.width() <= target.width())
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
    pub fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::EmptyDomain | Self::UnresolvedWidth => {
                "type-integer-representation-width-invalid"
            }
            Self::UnboundedDomain => "type-integer-refinement-requires-finite-bounds",
            Self::RuntimeWidthUnsupported(_) => "type-integer-runtime-width-unsupported",
            Self::RepresentationTooSmall { .. } => "type-integer-representation-too-small",
        }
    }
}

pub fn resolve_representation(
    validity: &IntegerValidity,
    rule: &IntegerRepresentationRule,
) -> Result<IntegerRepresentation, IntegerRepresentationError> {
    let required = validity
        .domain()
        .storage_hull()
        .map(FiniteHull::inferred_representation)
        .ok_or_else(|| match validity.domain().predicate() {
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
    validity: &IntegerValidity,
    rule: &IntegerRepresentationRule,
) -> Result<IntegerRepresentation, IntegerRepresentationError> {
    let representation = resolve_representation(validity, rule)?;
    if !representation.is_runtime_supported() {
        return Err(IntegerRepresentationError::RuntimeWidthUnsupported(
            representation,
        ));
    }
    Ok(representation)
}
