use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::ControlFlow;

use ast::{BinderId, ConstValue, GroupPath, NormalExpr};

use ast::HashFloat;
use ast::parameter::{ParamConvention, ParameterKind, Parameters};
use ast::prelude::*;
use ast::source::SourceExt;
use ast::span::{SpanId, SpanTable, SubSpan};
use ast::ty::ParamKind;
use ast::ty::PredicateExpr;
use ast_format::type_expr::ExprFormatExt;
use derive_more::From;
use diagnostic::Diagnostic;
use internment::Intern;

use crate::solver::{ConstraintEnv, Predicate, ProveResult};
use crate::ty::{Ty, TyArg};

/// Opaque file identifier assigned during compilation coordination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct FileId(pub u32);

/// A definition (bind) identifier — the fully-qualified name.
/// Interned string, e.g. Intern("main") or Intern("Range.new").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct DefId(pub Intern<String>);

/// A tag (type) identifier — the interned tag name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct TagId(pub Intern<String>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VariantId {
    pub union: TagId,
    pub name: Intern<String>,
}

/// Index into the expression arena (soa_derive TypedExprVec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct ExprId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct PlaceId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct PlaceVersionId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct TypedPlace {
    pub binder: BinderId,
    pub name: Intern<String>,
    pub parent: Option<PlaceId>,
    pub projection: Option<PlaceProjection>,
    pub mutable: bool,
    pub explicit_contract: bool,
    pub ty: Ty,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlaceProjection {
    Field(usize),
    Item(TargetIndex),
    Deref,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlaceVersionOrigin {
    Parameter,
    Declared,
    Initializer(ExprId),
    Projection {
        base: PlaceVersionId,
    },
    Rebind {
        value: ExprId,
        predecessor: Option<PlaceVersionId>,
    },
    Join(Vec<PlaceVersionId>),
    LoopPhi {
        incoming: PlaceVersionId,
        backedges: Vec<PlaceVersionId>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedPlaceVersion {
    pub place: PlaceId,
    pub origin: PlaceVersionOrigin,
    pub ty: Ty,
    pub integer_knowledge: Option<ast::integer::IntegerKnowledge>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaceVersionComponent {
    pub versions: Vec<PlaceVersionId>,
    pub cyclic: bool,
}

impl ExprId {
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Availability {
    Unknown,
    CompileTime,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvailabilityRequirement {
    Any,
    CompileTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallCapability {
    StagePolymorphic,
    RuntimeOnly,
}

pub use crate::intrinsic::IntrinsicOp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct GroupId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlap {
    Disjoint,
    Equal,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TargetIndex {
    Symbolic(NormalExpr),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EffectTarget {
    Group(GroupId),
    Field {
        base: Box<EffectTarget>,
        index: usize,
    },
    Variant {
        base: Box<EffectTarget>,
        name: Intern<String>,
    },
    ItemRegion {
        base: Box<EffectTarget>,
        index: TargetIndex,
    },
    ItemRange {
        base: Box<EffectTarget>,
        start: TargetIndex,
        end: TargetIndex,
    },
    Deref(Box<EffectTarget>),
    Descendants(Box<EffectTarget>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedEffectTarget {
    pub target: ReferenceTargetGroup,
    pub descendants: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReferenceTargetGroup {
    Param(GroupId),
    Local(PlaceId),
    Field {
        base: Box<ReferenceTargetGroup>,
        index: usize,
    },
    Variant {
        base: Box<ReferenceTargetGroup>,
        name: Intern<String>,
    },
    ItemRegion {
        base: Box<ReferenceTargetGroup>,
        index: TargetIndex,
    },
    ItemRange {
        base: Box<ReferenceTargetGroup>,
        start: TargetIndex,
        end: TargetIndex,
    },
    Deref(Box<ReferenceTargetGroup>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceTargetSet {
    targets: HashSet<ReferenceTargetGroup>,
}

impl ReferenceTargetSet {
    pub fn singleton(target: ReferenceTargetGroup) -> Self {
        Self {
            targets: HashSet::from([target]),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &ReferenceTargetGroup> {
        self.targets.iter()
    }

    pub fn extend(&mut self, other: &Self) {
        self.targets.extend(other.targets.iter().cloned());
    }

    pub fn projected(
        &self,
        mut project: impl FnMut(&ReferenceTargetGroup) -> ReferenceTargetGroup,
    ) -> Self {
        Self {
            targets: self.targets.iter().map(&mut project).collect(),
        }
    }

    pub fn is_disjoint(&self, other: &Self, constraints: &ConstraintEnv) -> bool {
        self.targets.iter().all(|left| {
            other
                .targets
                .iter()
                .all(|right| left.overlap(right, constraints) == Overlap::Disjoint)
        })
    }

    pub fn is_derived_from(
        &self,
        parent: &Self,
        projections: &[GroupProjection],
        constraints: &ConstraintEnv,
    ) -> bool {
        self.targets.iter().all(|target| {
            parent
                .targets
                .iter()
                .any(|parent| target.matches_projection_from(parent, projections, constraints))
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TargetGroupApplication {
    actuals: HashMap<GroupId, ReferenceTargetSet>,
    grouped_argument_counts: HashMap<GroupId, usize>,
    grouped_argument_mutate: HashMap<GroupId, bool>,
    grouped_argument_observe: HashMap<GroupId, bool>,
}

impl TargetGroupApplication {
    pub fn targets(&self, group: GroupId) -> Option<&ReferenceTargetSet> {
        self.actuals.get(&group)
    }

    pub fn invalid_derived_groups(
        &self,
        signature: &TypedCallableSignature,
        constraints: &ConstraintEnv,
    ) -> Vec<(GroupId, GroupId)> {
        let mut invalid = Vec::new();
        for (group, actuals) in &self.actuals {
            let Some(derived_group) = signature.groups.get(group.0 as usize) else {
                continue;
            };
            let (Some(path), Some(projections)) = (
                derived_group.path.as_ref(),
                derived_group.projections.as_ref(),
            ) else {
                continue;
            };
            let ancestor = signature
                .groups
                .iter()
                .enumerate()
                .filter_map(|(index, candidate)| {
                    let candidate = candidate.path.as_ref()?;
                    candidate
                        .is_ancestor_of(path)
                        .then_some((GroupId(index as u32), candidate.segments.len()))
                })
                .max_by_key(|(_, depth)| *depth);
            let Some((ancestor, ancestor_depth)) = ancestor else {
                continue;
            };
            let Some(parent_actuals) = self.actuals.get(&ancestor) else {
                continue;
            };
            if !actuals.is_derived_from(parent_actuals, &projections[ancestor_depth..], constraints)
            {
                invalid.push((*group, ancestor));
            }
        }
        invalid.sort_by_key(|(group, ancestor)| (group.0, ancestor.0));
        invalid
    }

    pub fn overlapping_mutated_pairs(
        &self,
        signature: &TypedCallableSignature,
        constraints: &ConstraintEnv,
    ) -> Vec<(GroupId, GroupId)> {
        let mut conflicts = Vec::new();
        let mut groups: Vec<_> = self.actuals.keys().copied().collect();

        for group in self.actuals.keys() {
            let count = self
                .grouped_argument_counts
                .get(group)
                .copied()
                .unwrap_or(0);
            let has_mutate = self
                .grouped_argument_mutate
                .get(group)
                .copied()
                .unwrap_or(false);
            let has_observe = self
                .grouped_argument_observe
                .get(group)
                .copied()
                .unwrap_or(false);
            if count > 1 && has_mutate && has_observe {
                conflicts.push((*group, *group));
            }
        }

        groups.sort_by_key(|group| group.0);
        for (index, left) in groups.iter().enumerate() {
            for right in &groups[index + 1..] {
                if !(signature.group_is_mutated(*left) || signature.group_is_mutated(*right)) {
                    continue;
                }
                if signature.allows_parent_child_overlap(*left, *right) {
                    continue;
                }
                let left_targets = &self.actuals[left];
                let right_targets = &self.actuals[right];
                if !left_targets.is_disjoint(right_targets, constraints) {
                    conflicts.push((*left, *right));
                }
            }
        }
        conflicts.sort_by_key(|(left, right)| (left.0, right.0));
        conflicts.dedup();
        conflicts
    }
}

impl<const N: usize> From<[ReferenceTargetGroup; N]> for ReferenceTargetSet {
    fn from(targets: [ReferenceTargetGroup; N]) -> Self {
        Self {
            targets: HashSet::from(targets),
        }
    }
}

impl EffectTarget {
    pub fn from_reference(target: &ReferenceTargetGroup) -> Option<Self> {
        match target {
            ReferenceTargetGroup::Param(group) => Some(Self::Group(*group)),
            ReferenceTargetGroup::Field { base, index } => Some(Self::Field {
                base: Box::new(Self::from_reference(base)?),
                index: *index,
            }),
            ReferenceTargetGroup::Variant { base, name } => Some(Self::Variant {
                base: Box::new(Self::from_reference(base)?),
                name: *name,
            }),
            ReferenceTargetGroup::ItemRegion { base, index } => Some(Self::ItemRegion {
                base: Box::new(Self::from_reference(base)?),
                index: index.clone(),
            }),
            ReferenceTargetGroup::ItemRange { base, start, end } => Some(Self::ItemRange {
                base: Box::new(Self::from_reference(base)?),
                start: start.clone(),
                end: end.clone(),
            }),
            ReferenceTargetGroup::Deref(base) => {
                Some(Self::Deref(Box::new(Self::from_reference(base)?)))
            }
            ReferenceTargetGroup::Local(_) => None,
        }
    }

    pub fn substitute(
        &self,
        group: GroupId,
        actual: &ReferenceTargetGroup,
    ) -> Option<AppliedEffectTarget> {
        let (target, descendants) = self.substitute_inner(group, actual)?;
        Some(AppliedEffectTarget {
            target,
            descendants,
        })
    }

    fn substitute_inner(
        &self,
        group: GroupId,
        actual: &ReferenceTargetGroup,
    ) -> Option<(ReferenceTargetGroup, bool)> {
        match self {
            Self::Group(effect_group) => (*effect_group == group).then(|| (actual.clone(), false)),
            Self::Field { base, index } => {
                let (base, descendants) = base.substitute_inner(group, actual)?;
                Some((
                    ReferenceTargetGroup::Field {
                        base: Box::new(base),
                        index: *index,
                    },
                    descendants,
                ))
            }
            Self::Variant { base, name } => {
                let (base, descendants) = base.substitute_inner(group, actual)?;
                Some((
                    ReferenceTargetGroup::Variant {
                        base: Box::new(base),
                        name: *name,
                    },
                    descendants,
                ))
            }
            Self::ItemRegion { base, index } => {
                let (base, descendants) = base.substitute_inner(group, actual)?;
                Some((
                    ReferenceTargetGroup::ItemRegion {
                        base: Box::new(base),
                        index: index.clone(),
                    },
                    descendants,
                ))
            }
            Self::ItemRange { base, start, end } => {
                let (base, descendants) = base.substitute_inner(group, actual)?;
                Some((
                    ReferenceTargetGroup::ItemRange {
                        base: Box::new(base),
                        start: start.clone(),
                        end: end.clone(),
                    },
                    descendants,
                ))
            }
            Self::Deref(base) => {
                let (base, descendants) = base.substitute_inner(group, actual)?;
                Some((ReferenceTargetGroup::Deref(Box::new(base)), descendants))
            }
            Self::Descendants(base) => {
                let (target, _) = base.substitute_inner(group, actual)?;
                Some((target, true))
            }
        }
    }
}

impl ReferenceTargetGroup {
    pub fn root_param(&self) -> Option<GroupId> {
        match self {
            Self::Param(group) => Some(*group),
            Self::Local(_) => None,
            Self::Field { base, .. }
            | Self::Variant { base, .. }
            | Self::ItemRegion { base, .. }
            | Self::ItemRange { base, .. }
            | Self::Deref(base) => base.root_param(),
        }
    }

    pub fn matches_projection_from(
        &self,
        ancestor: &Self,
        projections: &[GroupProjection],
        constraints: &ConstraintEnv,
    ) -> bool {
        let Some((projection, prefix)) = projections.split_last() else {
            return self.overlap(ancestor, constraints) == Overlap::Equal;
        };
        match projection {
            GroupProjection::Field { index, .. } => match self {
                Self::Field {
                    base,
                    index: actual_index,
                } if actual_index == index => {
                    base.matches_projection_from(ancestor, prefix, constraints)
                }
                _ => false,
            },
            GroupProjection::Items => match self {
                Self::ItemRegion { base, .. } | Self::ItemRange { base, .. } => {
                    base.matches_projection_from(ancestor, prefix, constraints)
                }
                _ => false,
            },
            GroupProjection::Pointee => match self {
                Self::Deref(base) => base.matches_projection_from(ancestor, prefix, constraints),
                _ => false,
            },
            GroupProjection::Variant { name } => match self {
                Self::Variant {
                    base,
                    name: actual_name,
                } if actual_name == name => {
                    base.matches_projection_from(ancestor, prefix, constraints)
                }
                _ => false,
            },
        }
    }

    pub fn is_strict_descendant_of(&self, ancestor: &Self, constraints: &ConstraintEnv) -> bool {
        let mut current = self.parent();
        while let Some(target) = current {
            if target.overlap(ancestor, constraints) == Overlap::Equal {
                return true;
            }
            current = target.parent();
        }
        false
    }

    pub fn overlap(&self, other: &Self, constraints: &ConstraintEnv) -> Overlap {
        match (self, other) {
            (Self::Param(left), Self::Param(right)) => {
                if left == right {
                    Overlap::Equal
                } else {
                    Overlap::Disjoint
                }
            }
            (Self::Local(left), Self::Local(right)) => {
                if left == right {
                    Overlap::Equal
                } else {
                    Overlap::Disjoint
                }
            }
            (
                Self::Field {
                    base: left_base,
                    index: left_index,
                },
                Self::Field {
                    base: right_base,
                    index: right_index,
                },
            ) => match left_base.overlap(right_base, constraints) {
                Overlap::Disjoint => Overlap::Disjoint,
                Overlap::Equal if left_index == right_index => Overlap::Equal,
                Overlap::Equal => Overlap::Disjoint,
                Overlap::Partial => Overlap::Partial,
                Overlap::Unknown => Overlap::Unknown,
            },
            (
                Self::Variant {
                    base: left_base,
                    name: left_name,
                },
                Self::Variant {
                    base: right_base,
                    name: right_name,
                },
            ) => match left_base.overlap(right_base, constraints) {
                Overlap::Disjoint => Overlap::Disjoint,
                Overlap::Equal if left_name == right_name => Overlap::Equal,
                Overlap::Equal => Overlap::Disjoint,
                Overlap::Partial => Overlap::Partial,
                Overlap::Unknown => Overlap::Unknown,
            },
            (
                Self::ItemRegion {
                    base: left_base,
                    index: left_index,
                },
                Self::ItemRegion {
                    base: right_base,
                    index: right_index,
                },
            ) => match left_base.overlap(right_base, constraints) {
                Overlap::Disjoint => Overlap::Disjoint,
                Overlap::Equal => index_overlap(left_index, right_index, constraints),
                Overlap::Partial => Overlap::Partial,
                Overlap::Unknown => Overlap::Unknown,
            },
            (
                Self::ItemRegion {
                    base: item_base,
                    index,
                },
                Self::ItemRange {
                    base: range_base,
                    start,
                    end,
                },
            ) => match item_base.overlap(range_base, constraints) {
                Overlap::Disjoint => Overlap::Disjoint,
                Overlap::Equal => index_range_overlap(index, start, end, constraints),
                Overlap::Partial => Overlap::Partial,
                Overlap::Unknown => Overlap::Unknown,
            },
            (Self::ItemRange { .. }, Self::ItemRegion { .. }) => other.overlap(self, constraints),
            (
                Self::ItemRange {
                    base: left_base,
                    start: left_start,
                    end: left_end,
                },
                Self::ItemRange {
                    base: right_base,
                    start: right_start,
                    end: right_end,
                },
            ) => match left_base.overlap(right_base, constraints) {
                Overlap::Disjoint => Overlap::Disjoint,
                Overlap::Equal => {
                    range_overlap(left_start, left_end, right_start, right_end, constraints)
                }
                Overlap::Partial => Overlap::Partial,
                Overlap::Unknown => Overlap::Unknown,
            },
            (Self::Deref(left), Self::Deref(right)) => left.overlap(right, constraints),
            _ => match self.depth().cmp(&other.depth()) {
                std::cmp::Ordering::Greater => self
                    .parent()
                    .map(|parent| descendant_overlap(parent.overlap(other, constraints)))
                    .unwrap_or(Overlap::Disjoint),
                std::cmp::Ordering::Less => other
                    .parent()
                    .map(|parent| descendant_overlap(self.overlap(parent, constraints)))
                    .unwrap_or(Overlap::Disjoint),
                std::cmp::Ordering::Equal => match (self.parent(), other.parent()) {
                    (Some(left), Some(right)) => {
                        descendant_overlap(left.overlap(right, constraints))
                    }
                    _ => Overlap::Disjoint,
                },
            },
        }
    }

    fn depth(&self) -> usize {
        self.parent().map_or(0, |parent| parent.depth() + 1)
    }

    fn parent(&self) -> Option<&Self> {
        match self {
            Self::Field { base, .. }
            | Self::Variant { base, .. }
            | Self::ItemRegion { base, .. }
            | Self::ItemRange { base, .. }
            | Self::Deref(base) => Some(base),
            Self::Param(_) | Self::Local(_) => None,
        }
    }
}

fn descendant_overlap(overlap: Overlap) -> Overlap {
    match overlap {
        Overlap::Equal | Overlap::Partial => Overlap::Partial,
        other => other,
    }
}

fn index_overlap(left: &TargetIndex, right: &TargetIndex, constraints: &ConstraintEnv) -> Overlap {
    let (TargetIndex::Symbolic(left), TargetIndex::Symbolic(right)) = (left, right) else {
        return Overlap::Unknown;
    };
    let values = HashMap::new();
    let equality = constraints.prove(&Predicate::Eq(left.clone(), right.clone()), &values);
    let equality = if equality == ProveResult::Unknown {
        constraints.prove(&Predicate::Eq(right.clone(), left.clone()), &values)
    } else {
        equality
    };
    match equality {
        ProveResult::Proven => Overlap::Equal,
        ProveResult::Disproven => Overlap::Disjoint,
        ProveResult::Unknown => {
            let ordered = constraints.prove(&Predicate::Lt(left.clone(), right.clone()), &values)
                == ProveResult::Proven
                || constraints.prove(&Predicate::Lt(right.clone(), left.clone()), &values)
                    == ProveResult::Proven;
            if ordered {
                Overlap::Disjoint
            } else {
                Overlap::Unknown
            }
        }
    }
}

fn index_range_overlap(
    index: &TargetIndex,
    start: &TargetIndex,
    end: &TargetIndex,
    constraints: &ConstraintEnv,
) -> Overlap {
    let (TargetIndex::Symbolic(index), TargetIndex::Symbolic(start), TargetIndex::Symbolic(end)) =
        (index, start, end)
    else {
        return Overlap::Unknown;
    };
    let values = HashMap::new();
    if constraints.prove(&Predicate::Lt(index.clone(), start.clone()), &values)
        == ProveResult::Proven
        || constraints.prove(&Predicate::Le(end.clone(), index.clone()), &values)
            == ProveResult::Proven
    {
        return Overlap::Disjoint;
    }
    if constraints.prove(&Predicate::Le(start.clone(), index.clone()), &values)
        == ProveResult::Proven
        && constraints.prove(&Predicate::Lt(index.clone(), end.clone()), &values)
            == ProveResult::Proven
    {
        return Overlap::Partial;
    }
    Overlap::Unknown
}

fn range_overlap(
    left_start: &TargetIndex,
    left_end: &TargetIndex,
    right_start: &TargetIndex,
    right_end: &TargetIndex,
    constraints: &ConstraintEnv,
) -> Overlap {
    let (
        TargetIndex::Symbolic(left_start),
        TargetIndex::Symbolic(left_end),
        TargetIndex::Symbolic(right_start),
        TargetIndex::Symbolic(right_end),
    ) = (left_start, left_end, right_start, right_end)
    else {
        return Overlap::Unknown;
    };
    let values = HashMap::new();
    if constraints.prove(
        &Predicate::Le(left_end.clone(), right_start.clone()),
        &values,
    ) == ProveResult::Proven
        || constraints.prove(
            &Predicate::Le(right_end.clone(), left_start.clone()),
            &values,
        ) == ProveResult::Proven
    {
        return Overlap::Disjoint;
    }
    let starts_equal = constraints.prove(
        &Predicate::Eq(left_start.clone(), right_start.clone()),
        &values,
    ) == ProveResult::Proven;
    let ends_equal = constraints
        .prove(&Predicate::Eq(left_end.clone(), right_end.clone()), &values)
        == ProveResult::Proven;
    if starts_equal && ends_equal {
        return Overlap::Equal;
    }
    let intersects = constraints.prove(
        &Predicate::Lt(left_start.clone(), right_end.clone()),
        &values,
    ) == ProveResult::Proven
        && constraints.prove(
            &Predicate::Lt(right_start.clone(), left_end.clone()),
            &values,
        ) == ProveResult::Proven;
    if intersects {
        Overlap::Partial
    } else {
        Overlap::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GroupProjection {
    Field { name: Intern<String>, index: usize },
    Items,
    Pointee,
    Variant { name: Intern<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedGroup {
    pub path: Option<GroupPath>,
    pub projections: Option<Vec<GroupProjection>>,
    pub referent_type: Ty,
}

/// Type alias for variant map entries: (union_name, discriminant, fields)
pub type VariantMapEntry = (Intern<String>, usize, Vec<(Intern<String>, Ty)>);

/// Type alias for the variant map: variant_name -> [(union_name, discriminant, fields)]
pub type VariantMap = HashMap<Intern<String>, Vec<VariantMapEntry>>;

/// Type alias for variant lookup result: (union_name, discriminant, field_slice)
pub type VariantLookupResult<'a> = (Intern<String>, usize, &'a [(Intern<String>, Ty)]);

/// Merge per-file variant maps once for package-scoped IDE / cross-file lowering.
///
/// Each file only stores variants for tags it declares.
pub fn collect_package_variant_map(asts: &[&TypedFileAst]) -> VariantMap {
    let mut seen: std::collections::HashSet<(Intern<String>, Intern<String>, usize)> =
        std::collections::HashSet::new();
    let mut variant_map: VariantMap = HashMap::new();
    for ast in asts {
        for (variant_name, entries) in &ast.variant_map {
            for entry in entries {
                let (union_name, disc, _) = entry;
                if !seen.insert((*variant_name, *union_name, *disc)) {
                    continue;
                }
                variant_map
                    .entry(*variant_name)
                    .or_default()
                    .push(entry.clone());
            }
        }
    }
    variant_map
}

use soa_derive::StructOfArray;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MathematicalComparison {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
    NotEqual,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EvidenceProposition {
    IntegerComparison {
        lhs: ExprId,
        rhs: ExprId,
        comparison: MathematicalComparison,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AlternativeEvidence {
    pub label: Intern<String>,
    pub proposition: EvidenceProposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResultEvidence {
    pub owner: ast::ResultFamilyOwner,
    pub alternatives: Vec<AlternativeEvidence>,
}

pub type ProjectedResultEvidence = HashMap<Vec<usize>, ResultEvidence>;

/// A single expression in the typed AST arena.
///
/// All fields are stored in separate vectors via `soa_derive` for cache-friendly
/// iteration and per-field access.
#[derive(Debug, Clone, PartialEq, StructOfArray)]
#[soa_derive(Debug, Clone, PartialEq)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub ty: Ty,
    /// Source location for diagnostics and LSP.
    pub span: SpanId,
    /// Compile-time constant value, if this expression can be folded.
    pub const_value: Option<ast::ConstValue>,
    pub integer_knowledge: Option<ast::integer::IntegerKnowledge>,
    pub result_evidence: Option<ResultEvidence>,
    pub projected_result_evidence: ProjectedResultEvidence,
    /// Staging availability of this expression.
    pub availability: Availability,
    pub target_group: Option<ReferenceTargetSet>,
    pub place: Option<PlaceId>,
    pub place_version: Option<PlaceVersionId>,
    /// Type/flow/flaw diagnostics attached to this expression.
    pub flaws: Vec<Diagnostic>,
}

impl TypedExprVec {
    #[inline]
    pub fn kind_of(&self, expr_id: ExprId) -> Option<&TypedExprKind> {
        self.kind.get(expr_id.index())
    }

    #[inline]
    pub fn ty_of(&self, expr_id: ExprId) -> Option<&Ty> {
        self.ty.get(expr_id.index())
    }

    #[inline]
    pub fn span_of(&self, expr_id: ExprId) -> Option<SpanId> {
        self.span.get(expr_id.index()).copied()
    }

    #[inline]
    pub fn const_value_of(&self, expr_id: ExprId) -> Option<&Option<ast::ConstValue>> {
        self.const_value.get(expr_id.index())
    }

    #[inline]
    pub fn integer_knowledge_of(
        &self,
        expr_id: ExprId,
    ) -> Option<&Option<ast::integer::IntegerKnowledge>> {
        self.integer_knowledge.get(expr_id.index())
    }

    #[inline]
    pub fn result_evidence_of(&self, expr_id: ExprId) -> Option<&Option<ResultEvidence>> {
        self.result_evidence.get(expr_id.index())
    }

    #[inline]
    pub fn projected_result_evidence_of(
        &self,
        expr_id: ExprId,
    ) -> Option<&ProjectedResultEvidence> {
        self.projected_result_evidence.get(expr_id.index())
    }

    #[inline]
    pub fn flaws_of(&self, expr_id: ExprId) -> Option<&Vec<Diagnostic>> {
        self.flaws.get(expr_id.index())
    }

    #[inline]
    pub fn availability_of(&self, expr_id: ExprId) -> Option<&Availability> {
        self.availability.get(expr_id.index())
    }

    #[inline]
    pub fn target_group_of(&self, expr_id: ExprId) -> Option<&Option<ReferenceTargetSet>> {
        self.target_group.get(expr_id.index())
    }
}

/// Typed expression variant — post-resolution form of parse-time [`Expr`].
///
/// Key differences from parse-time `Expr`:
/// - `TypeNominal`, `TypeQualified`, `TypeGeneric` — removed (desugared to `Ty`)
/// - `AnonymousTag` — removed (merged into `TagCall` with `args: None`)
/// - `FnCall` — uses `DefId` instead of path
/// - `TagCall` — uses `VariantId` + discriminant
/// - `Cast` — `ty` is `Ty` not `Intern<String>`
/// - All `Box<Typed<Expr>>` → `ExprId`
/// - All `Vec<Typed<Expr>>` → `Vec<ExprId>`
///
/// Typed when-expression — like `WhenExpr` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedWhenExpr {
    /// Subject expression for pattern matching (`None` for condition-based when).
    pub subject: Option<ExprId>,
    pub arms: Vec<TypedWhenArm>,
    pub place_joins: Vec<PlaceVersionId>,
    /// Covers from after the `when` keyword to end.
    /// The full expression span is on the `TypedExpr` arena entry.
    pub body_span: SubSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedWhenArm {
    Cond {
        condition: TypedCondition,
        body: ExprId,
        /// Span of this arm (condition and body).
        arm_span: SubSpan,
    },
    Is {
        pattern: Box<ast::span::Spanned<ast::Pattern>>,
        body: ExprId,
        /// Span of this is-arm (pattern and body).
        arm_span: SubSpan,
    },
    Else(ExprId, SubSpan),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedCondition {
    Is {
        subject: ExprId,
        pattern: Box<ast::span::Spanned<ast::Pattern>>,
    },
    Not(Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
}

impl TypedCondition {
    pub fn visit_subjects(&self, visit: &mut impl FnMut(ExprId)) {
        match self {
            Self::Is { subject, .. } => visit(*subject),
            Self::Not(inner) => inner.visit_subjects(visit),
            Self::And(left, right) | Self::Or(left, right) => {
                left.visit_subjects(visit);
                right.visit_subjects(visit);
            }
        }
    }

    pub fn pattern_subjects<'a>(
        &'a self,
        output: &mut Vec<(ExprId, &'a ast::span::Spanned<ast::Pattern>)>,
    ) {
        match self {
            Self::Is { subject, pattern } => output.push((*subject, pattern)),
            Self::Not(inner) => inner.pattern_subjects(output),
            Self::And(left, right) | Self::Or(left, right) => {
                left.pattern_subjects(output);
                right.pattern_subjects(output);
            }
        }
    }
}

/// Typed if-expression — like `IfExpr` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedIfExpr {
    pub condition: TypedCondition,
    pub stmts: Vec<ExprId>,
    pub ret: Option<ExprId>,
    pub place_joins: Vec<PlaceVersionId>,
    /// Covers from condition start to end (excludes the `if` keyword).
    /// The full expression span (including `if`) is on the `TypedExpr` arena entry.
    pub body_span: SubSpan,
}

/// Typed loop — like `Loop` but with `ExprId` children.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedLoop {
    pub kind: TypedLoopKind,
    pub stmts: Vec<ExprId>,
    pub place_phis: Vec<PlaceVersionId>,
    /// Span of the `loop` keyword only.
    /// The full expression span is on the `TypedExpr` arena entry.
    pub keyword_span: SubSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedLoopKind {
    While {
        condition: TypedCondition,
    },
    ForIn {
        variable: Intern<String>,
        iterable: ExprId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedExprKind {
    Lit(Literal),
    Binary {
        op: BinOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    InvalidOperator {
        role: OperatorRole,
        lhs: ExprId,
        rhs: ExprId,
        error: crate::operator::OperatorResolutionError,
    },
    FnCall {
        target: DefId,
        args: Option<Vec<ExprId>>,
        operator_role: Option<OperatorRole>,
        /// When the target has const-generic params, the return type after
        /// substituting type/const arguments from the call site.
        substituted_ty: Option<Ty>,
    },
    IntrinsicCall {
        op: IntrinsicOp,
        args: Vec<ExprId>,
    },
    TagCall {
        variant_id: VariantId,
        discriminant: usize,
        args: Option<Vec<ExprId>>,
        /// Original field names from the AST TagCall args (for named/positional
        /// args in shape literals). Empty for bare anonymous tags.
        field_names: Vec<Intern<String>>,
    },
    Bind {
        name: Intern<String>,
        stmts: Vec<ExprId>,
        body: ExprId,
        /// If true, this bind was declared with a type but no value (`name Type`).
        /// The variable starts in `Declared` state and must be assigned before use.
        unassigned: bool,
    },
    /// Reassign a value to a previously-declared variable.
    Reassign {
        name: Intern<String>,
        value: ExprId,
        operator: Option<BinOp>,
    },
    When(TypedWhenExpr),
    If(TypedIfExpr),
    Loop(TypedLoop),
    SelfRef {
        target: DefId,
    },
    FormatString(FormatString),
    Range {
        start: ExprId,
        end: ExprId,
    },
    TupleLit(Vec<ExprId>),
    List(Vec<ExprId>),

    Cast {
        expr: ExprId,
        ty: Ty,
    },
    TargetQuery {
        kind: ast::TargetQueryKind,
        operand: Ty,
    },
    Rematerialize {
        source: ExprId,
        constant: ConstValue,
    },

    TupleAlloc {
        init: ExprId,
        size: NormalExpr,
    },
    TupleGet {
        base: ExprId,
        index: usize,
    },
    TupleSet {
        base: ExprId,
        index: usize,
        value: ExprId,
        operator: Option<ResolvedCompoundOperator>,
    },
    /// Destructure bind: `Tag(field: bind, …) := expr`
    Destructure {
        tag_name: Intern<String>,
        value: ExprId,
        field_bindings: Vec<(Intern<String>, Intern<String>)>,
    },

    /// Record field write: `base.field:: value`
    RecordSet {
        base: ExprId,
        field: Intern<String>,
        value: ExprId,
        operator: Option<ResolvedCompoundOperator>,
    },
    BufGet {
        buf: ExprId,
        index: ExprId,
    },
    BufSet {
        buf: ExprId,
        index: ExprId,
        value: ExprId,
        operator: Option<ResolvedCompoundOperator>,
    },
    TakePtr(ExprId),
    /// A safe reference: `ref expr` or `mut expr`.
    Ref(ExprId),
    Deref(ExprId),

    Negate(ExprId),

    /// Argument passed with `eat` at call site: explicit consume.
    ConsumeArg(ExprId),
    /// Explicit consume: `eat expr`.
    Eat(ExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedCompoundOperator {
    Callable { target: DefId, return_ty: Ty },
    Intrinsic(IntrinsicOp),
    Invalid(crate::operator::OperatorResolutionError),
}

pub fn walk_expr_children(
    kind: &TypedExprKind,
    visit: &mut impl FnMut(ExprId) -> std::ops::ControlFlow<()>,
) -> std::ops::ControlFlow<()> {
    use std::ops::ControlFlow::Continue;

    match kind {
        TypedExprKind::Binary { lhs, rhs, .. }
        | TypedExprKind::InvalidOperator { lhs, rhs, .. } => {
            visit(*lhs)?;
            visit(*rhs)?;
        }
        TypedExprKind::IntrinsicCall { args, .. } => {
            for child in args {
                visit(*child)?;
            }
        }
        TypedExprKind::FnCall { args, .. } | TypedExprKind::TagCall { args, .. } => {
            if let Some(args) = args {
                for child in args {
                    visit(*child)?;
                }
            }
        }
        TypedExprKind::Bind {
            stmts,
            body,
            unassigned,
            ..
        } => {
            for child in stmts {
                visit(*child)?;
            }
            if !unassigned {
                visit(*body)?;
            }
        }
        TypedExprKind::Reassign { value, .. } | TypedExprKind::Destructure { value, .. } => {
            visit(*value)?
        }
        TypedExprKind::When(when_expr) => {
            if let Some(subject) = when_expr.subject {
                visit(subject)?;
            }
            for arm in &when_expr.arms {
                match arm {
                    TypedWhenArm::Cond {
                        condition, body, ..
                    } => {
                        let mut failure = None;
                        condition.visit_subjects(&mut |subject| {
                            if failure.is_none() {
                                failure = visit(subject).break_value();
                            }
                        });
                        if failure.is_some() {
                            return ControlFlow::Break(());
                        }
                        visit(*body)?;
                    }
                    TypedWhenArm::Is { body, .. } | TypedWhenArm::Else(body, _) => visit(*body)?,
                }
            }
        }
        TypedExprKind::If(if_expr) => {
            let mut failure = None;
            if_expr.condition.visit_subjects(&mut |subject| {
                if failure.is_none() {
                    failure = visit(subject).break_value();
                }
            });
            if failure.is_some() {
                return ControlFlow::Break(());
            }
            for child in &if_expr.stmts {
                visit(*child)?;
            }
            if let Some(child) = if_expr.ret {
                visit(child)?;
            }
        }
        TypedExprKind::Loop(loop_expr) => {
            match &loop_expr.kind {
                TypedLoopKind::While { condition, .. } => {
                    let mut failure = None;
                    condition.visit_subjects(&mut |subject| {
                        if failure.is_none() {
                            failure = visit(subject).break_value();
                        }
                    });
                    if failure.is_some() {
                        return ControlFlow::Break(());
                    }
                }
                TypedLoopKind::ForIn { iterable, .. } => visit(*iterable)?,
            }
            for child in &loop_expr.stmts {
                visit(*child)?;
            }
        }
        TypedExprKind::Range { start, end } => {
            visit(*start)?;
            visit(*end)?;
        }
        TypedExprKind::TupleLit(children) | TypedExprKind::List(children) => {
            for child in children {
                visit(*child)?;
            }
        }
        TypedExprKind::Cast { expr, .. }
        | TypedExprKind::Rematerialize { source: expr, .. }
        | TypedExprKind::TupleAlloc { init: expr, .. }
        | TypedExprKind::TupleGet { base: expr, .. }
        | TypedExprKind::TakePtr(expr)
        | TypedExprKind::Ref(expr)
        | TypedExprKind::Deref(expr)
        | TypedExprKind::Negate(expr)
        | TypedExprKind::ConsumeArg(expr)
        | TypedExprKind::Eat(expr) => visit(*expr)?,
        TypedExprKind::TupleSet { base, value, .. }
        | TypedExprKind::RecordSet { base, value, .. } => {
            visit(*base)?;
            visit(*value)?;
        }
        TypedExprKind::BufGet { buf, index } => {
            visit(*buf)?;
            visit(*index)?;
        }
        TypedExprKind::BufSet {
            buf, index, value, ..
        } => {
            visit(*buf)?;
            visit(*index)?;
            visit(*value)?;
        }
        TypedExprKind::Lit(_)
        | TypedExprKind::SelfRef { .. }
        | TypedExprKind::FormatString(_)
        | TypedExprKind::TargetQuery { .. } => {}
    }
    Continue(())
}

pub fn walk_expr_children_of(
    typed: &TypedFileAst,
    expr_id: ExprId,
    visit: &mut impl FnMut(ExprId) -> std::ops::ControlFlow<()>,
) -> std::ops::ControlFlow<()> {
    let Some(kind) = typed.exprs.kind_of(expr_id) else {
        return std::ops::ControlFlow::Continue(());
    };
    walk_expr_children(kind, visit)
}

pub fn walk_expr_preorder(
    typed: &TypedFileAst,
    root: ExprId,
    visit: &mut impl FnMut(ExprId) -> std::ops::ControlFlow<()>,
) -> std::ops::ControlFlow<()> {
    fn walk(
        typed: &TypedFileAst,
        expr_id: ExprId,
        visited: &mut std::collections::HashSet<ExprId>,
        visit: &mut impl FnMut(ExprId) -> std::ops::ControlFlow<()>,
    ) -> std::ops::ControlFlow<()> {
        if !visited.insert(expr_id) {
            return std::ops::ControlFlow::Continue(());
        }
        let Some(kind) = typed.exprs.kind.get(expr_id.as_usize()) else {
            return std::ops::ControlFlow::Continue(());
        };
        visit(expr_id)?;
        walk_expr_children(kind, &mut |child| walk(typed, child, visited, visit))
    }

    walk(typed, root, &mut std::collections::HashSet::new(), visit)
}

/// A fully-resolved tag declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedTag {
    pub name_span: SpanId,
    /// Full `Tag is …` / `Tag has …` site (for hover on the RHS, e.g. `in 0...255`).
    pub span: SpanId,
    /// The resolved type of this tag (e.g., `Ty::Union`, `Ty::Record`, `Ty::ConstUnion`, etc.).
    pub resolved_ty: Ty,
    /// Tag attributes (e.g., `#[test]`).
    pub attributes: DeclareAttributes,
    pub doc_comment: Option<DocComment>,
    /// Tag parameters (type variables, defaults), if any.
    pub params: Option<Parameters>,
    /// For record types (`has` bodies), field name → formatted type annotation surface
    /// (e.g. `"pointer"` → `"Pointer(x)"`). Populated during stage_declare.
    pub record_field_types: HashMap<Intern<String>, String>,
    /// For interface (`has`) bodies, method name → doc comment, if any.
    /// Only entries for members that have a doc are present.
    /// Populated during stage_declare alongside `record_field_types`.
    pub record_field_docs: HashMap<Intern<String>, String>,
    /// For record fields, field name → refinement predicate (e.g. `and < n`).
    /// Only entries for fields that have a refinement are present.
    pub record_field_refinements: HashMap<Intern<String>, PredicateExpr>,
    /// Formatted declaration text (e.g. "Bool is True or False"), for use in hover.
    pub declaration_text: String,
    /// Trait implementations provided via `and has TraitName(field: expr, ...)` clauses.
    /// Explicit (concrete) field definitions always win over these provided ones.
    pub provided_traits: Vec<ProvidedTrait>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FunctionEffects {
    pub reads: HashSet<EffectTarget>,
    pub writes: HashSet<EffectTarget>,
    pub invalidates: HashSet<EffectTarget>,
    pub consumes: HashSet<EffectTarget>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedCallableSignature {
    pub dependent_binder: BinderId,
    pub return_type: Ty,
    pub params: Vec<(Intern<String>, Ty)>,
    pub param_kinds: Vec<ParamKind>,
    pub param_conventions: Vec<ParamConvention>,
    pub param_groups: Vec<Option<GroupId>>,
    pub param_refinements: Vec<Option<PredicateExpr>>,
    pub groups: Vec<TypedGroup>,
    pub return_group: Option<GroupId>,
    pub effects: FunctionEffects,
    pub param_requirements: Vec<AvailabilityRequirement>,
    pub call_capability: CallCapability,
    pub intrinsic: Option<IntrinsicOp>,
    pub operator_role: Option<OperatorRole>,
}

impl TypedCallableSignature {
    pub fn group_is_mutated(&self, group: GroupId) -> bool {
        self.param_groups
            .iter()
            .zip(&self.param_conventions)
            .any(|(param_group, convention)| {
                *param_group == Some(group) && *convention == ParamConvention::Mutate
            })
    }

    pub fn allows_parent_child_overlap(&self, left: GroupId, right: GroupId) -> bool {
        let Some(left_path) = self
            .groups
            .get(left.0 as usize)
            .and_then(|group| group.path.as_ref())
        else {
            return false;
        };
        let Some(right_path) = self
            .groups
            .get(right.0 as usize)
            .and_then(|group| group.path.as_ref())
        else {
            return false;
        };

        (left_path.is_ancestor_of(right_path) && !self.group_is_mutated(left))
            || (right_path.is_ancestor_of(left_path) && !self.group_is_mutated(right))
    }
}

impl From<&TypedBind> for TypedCallableSignature {
    fn from(bind: &TypedBind) -> Self {
        Self {
            dependent_binder: bind.dependent_binder,
            return_type: bind.return_type.clone(),
            params: bind.params.clone(),
            param_kinds: bind.param_kinds.clone(),
            param_conventions: bind.param_conventions.clone(),
            param_groups: bind.param_groups.clone(),
            param_refinements: bind.param_refinements.clone(),
            groups: bind.groups.clone(),
            return_group: bind.return_group,
            effects: bind.effects.clone(),
            param_requirements: bind.param_requirements.clone(),
            call_capability: bind.call_capability,
            intrinsic: bind.intrinsic,
            operator_role: bind.operator_role,
        }
    }
}

/// A fully-resolved bind (function or value definition).
#[derive(Debug, Clone, PartialEq)]
pub struct TypedBind {
    pub name: Intern<String>,
    pub dependent_binder: BinderId,
    /// Span of the name in source.
    pub name_span: SpanId,
    pub body: BindBody,
    /// The resolved return type.
    pub return_type: Ty,
    /// User-written return type before later lowering.
    pub declared_return_type: Ty,
    pub return_evidence: Option<ResultEvidence>,
    /// Resolved parameter types (name, resolved Ty).
    pub params: Vec<(Intern<String>, Ty)>,
    /// Whether each parameter is a type or value parameter.
    pub param_kinds: Vec<ParamKind>,
    /// Ownership contract for each parameter, in declaration order.
    pub param_conventions: Vec<ParamConvention>,
    pub param_groups: Vec<Option<GroupId>>,
    pub param_refinements: Vec<Option<PredicateExpr>>,
    pub groups: Vec<TypedGroup>,
    pub return_group: Option<GroupId>,
    pub effects: FunctionEffects,
    pub param_requirements: Vec<AvailabilityRequirement>,
    pub call_capability: CallCapability,
    /// Receiver type for methods, if any.
    pub receiver_type: Option<Ty>,
    /// Bind attributes (e.g., `#[inline]`, visibility).
    pub attributes: BindAttributes,
    pub intrinsic: Option<IntrinsicOp>,
    pub operator_role: Option<OperatorRole>,
    pub doc_comment: Option<DocComment>,
    /// Bind-level diagnostics (e.g., redundant self-param type).
    pub flaws: Vec<Diagnostic>,
    /// `name Type` at module or function scope with no `:` value yet.
    pub unassigned_decl: bool,
    pub is_extern: bool,
    /// Bound with `:=` (immutable).
    pub is_constant: bool,
    /// Source-level signature for hover (types as written, not resolved).
    pub signature_surface: String,
}

/// The body of a [`TypedBind`].
#[derive(Debug, Clone, PartialEq)]
pub enum BindBody {
    Expr(ExprId),
    /// A block body with multiple expressions and an optional return expression.
    Body {
        exprs: Vec<ExprId>,
        ret: Option<ExprId>,
    },
    Extern,
}

/// The typed AST for one `.gin` file — all types resolved, all flaws attached.
///
/// This is the source of truth for LSP queries and further analysis.
#[derive(Clone)]
pub struct TypedFileAst {
    /// Span table mapping SpanId → byte ranges (cloned from FileAst).
    pub span_table: SpanTable,
    /// The file identifier assigned during compilation coordination.
    pub file_id: FileId,
    pub type_registry: crate::TypeRegistry,
    pub target_layout: Option<crate::layout::TargetLayout>,

    /// Resolved tag declarations.
    pub tags: HashMap<TagId, TypedTag>,
    /// Resolved bind (function/value) declarations.
    pub defs: HashMap<DefId, TypedBind>,
    /// Private tag names.
    pub private_tags: HashSet<TagId>,
    /// Private def names.
    pub private_defs: HashSet<DefId>,

    /// The expression arena — all expressions in SoA layout.
    pub exprs: TypedExprVec,

    /// Top-level expression IDs (e.g., standalone expressions in the file).
    pub root_exprs: Vec<ExprId>,
    pub places: Vec<TypedPlace>,
    pub place_versions: Vec<TypedPlaceVersion>,
    pub place_version_components: Vec<PlaceVersionComponent>,

    /// span.start byte offset → ExprId for O(log n) position-based lookup.
    pub span_to_expr: BTreeMap<u32, ExprId>,

    /// Raw import ModPaths from the source file's import statements.
    /// Used for module path hover detection (non-final segment → module doc).
    pub import_mod_paths: Vec<ast::Spanned<ast::ModPath>>,

    // (cache, deterministically reconstructible from declarations)
    /// Tag name → resolved type.
    pub tag_types: HashMap<TagId, Ty>,
    /// Function name → return type.
    pub fn_return_types: HashMap<DefId, Ty>,
    /// Variant name → [(union_name, discriminant, fields)].
    pub variant_map: VariantMap,
    /// Display text and doc comment for each variant, keyed by `"{union_tag}.{variant_name}"`.
    /// Populated during stage_declare from the AST `Variant` shapes.
    pub variant_annotations: HashMap<String, (String, Option<String>)>,
    pub imported_trait_names: HashSet<Intern<String>>,
    pub(crate) self_contexts: Vec<(SpanId, Intern<String>, Intern<String>)>,
    pub eval_ast: std::sync::Arc<ast::FileAst>,
    pub semantic_origin: Option<FileSemanticOrigin>,
    pub module_doc: Option<ast::DocComment>,
    /// Declaration-level warnings.
    pub warnings: Vec<Diagnostic>,
    /// Type-name flaws on declarations (`has` fields, return types, etc.).
    pub declaration_flaws: Vec<(SpanId, Diagnostic)>,
    /// Parse-level diagnostics from the parser (cursor errors, lex errors).
    pub parse_warnings: Vec<diagnostic::Diagnostic>,
}

impl std::fmt::Debug for TypedFileAst {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypedFileAst")
            .field("file_id", &self.file_id)
            .field("type_registry", &self.type_registry)
            .field("tags", &self.tags)
            .field("defs", &self.defs)
            .field("private_tags", &self.private_tags)
            .field("private_defs", &self.private_defs)
            .field("exprs", &self.exprs)
            .field("root_exprs", &self.root_exprs)
            .field("span_to_expr", &self.span_to_expr)
            .field("tag_types", &self.tag_types)
            .field("fn_return_types", &self.fn_return_types)
            .field("variant_map", &self.variant_map)
            .field("semantic_origin", &self.semantic_origin)
            .finish()
    }
}

impl PartialEq for TypedFileAst {
    fn eq(&self, other: &Self) -> bool {
        self.file_id == other.file_id
            && self.target_layout == other.target_layout
            && self.tags == other.tags
            && self.defs == other.defs
            && self.private_tags == other.private_tags
            && self.private_defs == other.private_defs
            && self.exprs == other.exprs
            && self.root_exprs == other.root_exprs
            && self.span_to_expr == other.span_to_expr
            && self.tag_types == other.tag_types
            && self.fn_return_types == other.fn_return_types
            && self.variant_map == other.variant_map
            && self.variant_annotations == other.variant_annotations
            && self.semantic_origin == other.semantic_origin
            && self.module_doc == other.module_doc
    }
}

#[derive(Copy, Clone)]
struct HoverPatternCtx<'a> {
    byte_offset: usize,
    word: &'a str,
    subject_ty: Option<&'a Ty>,
    tag_types: &'a HashMap<Intern<String>, Ty>,
    tag_params: Option<&'a HashMap<Intern<String>, Parameters>>,
    variant_map: &'a VariantMap,
    package: Option<&'a PackageSemanticIndex>,
}

/// What kind of thing is at a cursor position — used by [`TypedFileAst::classify_hover`].
#[derive(Debug, Clone)]
#[doc(hidden)]
pub enum HoverTarget {
    Definition(Intern<String>),
    Param {
        name: Intern<String>,
        surface: String,
    },
    SelfRef {
        /// e.g. "ref self", "mut self", "self", "eat self"
        modifier: Intern<String>,
        tag_name: Intern<String>,
    },
    TagDecl(Intern<String>),
    TagAtByte(Intern<String>),
    RecordField {
        name: Intern<String>,
        surface: String,
    },
    TypePattern(HoverResult),
    Variant {
        union_name: Intern<String>,
        discriminant: usize,
        union_ty: Ty,
    },
    Expr(ExprId),
    /// Cursor is on a non-final segment of a qualified module path.
    /// The string is the qualified module path (e.g. `"core.maybe"`).
    ModulePath(String),
}

impl TypedFileAst {
    pub fn apply_target_groups(
        &self,
        args: &[ExprId],
        signature: &TypedCallableSignature,
    ) -> TargetGroupApplication {
        let mut application = TargetGroupApplication::default();
        for (index, group) in signature.param_groups.iter().enumerate() {
            let Some(group) = group else {
                continue;
            };
            let Some(actual) = args
                .get(index)
                .and_then(|arg| self.exprs.target_group.get(arg.as_usize()))
                .and_then(|targets| targets.as_ref())
            else {
                continue;
            };
            let convention = signature
                .param_conventions
                .get(index)
                .copied()
                .unwrap_or(ParamConvention::Own);
            application
                .actuals
                .entry(*group)
                .or_default()
                .extend(actual);
            *application
                .grouped_argument_counts
                .entry(*group)
                .or_default() += 1;
            match convention {
                ParamConvention::Mutate => {
                    application.grouped_argument_mutate.insert(*group, true);
                }
                _ => {
                    application.grouped_argument_observe.insert(*group, true);
                }
            }
        }
        application
    }

    fn has_self_context_at_byte(
        &self,
        byte_offset: usize,
    ) -> Option<(Intern<String>, Intern<String>)> {
        if let Some((_, tag_name, modifier)) = self
            .self_contexts
            .iter()
            .find(|(span, _, _)| self.span_table.get(*span).contains(byte_offset))
        {
            return Some((*tag_name, *modifier));
        }

        let self_name = Intern::<String>::from_ref("self");
        for declare in self.eval_ast.tags.values() {
            let ast::DeclareValue::Has(members) = &declare.value else {
                continue;
            };
            for member in members {
                let ast::HasMember::Function(function) = member else {
                    continue;
                };
                if !function.params.contains_key(&self_name) {
                    continue;
                }
                let Some(body) = function.body.as_ref() else {
                    continue;
                };
                let value = match body {
                    ast::HasMemberBody::Overrideable(value) | ast::HasMemberBody::Final(value) => {
                        value
                    }
                };
                let mut self_spans = Vec::new();
                crate::transform::collect_bind_value_self_ref_spans(value, &mut self_spans);
                let ast_span_matches = self_spans
                    .iter()
                    .any(|span| self.span_table.get(*span).contains(byte_offset));
                if !ast_span_matches {
                    continue;
                }
                let modifier = match function.conventions.get(&self_name) {
                    Some(ast::ParamConvention::Observe) => "ref self",
                    Some(ast::ParamConvention::Mutate) => "mut self",
                    Some(ast::ParamConvention::Consume) => "eat self",
                    Some(ast::ParamConvention::Own) | None => "self",
                };
                return Some((declare.name, Intern::<String>::from_ref(modifier)));
            }
        }
        None
    }

    fn param_at_byte(&self, byte_offset: usize, word: &str) -> Option<(Intern<String>, String)> {
        let word = Intern::<String>::from_ref(word);
        let mut def_starts: Vec<usize> = self
            .defs
            .values()
            .map(|bind| self.span_table.get(bind.name_span).start())
            .collect();
        def_starts.sort_unstable();
        def_starts.dedup();
        for ast_bind in self.eval_ast.defs.values() {
            let Some(parameter) = ast_bind
                .params
                .as_ref()
                .and_then(|params| params.get(&word))
            else {
                continue;
            };
            let surface =
                self.defs
                    .get(&DefId(ast_bind.name))
                    .or_else(|| {
                        self.defs.values().find(|bind| {
                            bind.name.as_str() == ast_bind.name.as_str()
                                || bind.name.as_str().split('.').next_back()
                                    == Some(ast_bind.name.as_str())
                        })
                    })
                    .and_then(|typed_bind| {
                        signature_param_surface(&typed_bind.signature_surface, word.as_str())
                            .or_else(|| {
                                typed_bind.params.iter().find_map(|(name, ty)| {
                                    (*name == word).then(|| {
                                        type_annotation_surface_for_hover_local(
                                            ty,
                                            &self.tag_types,
                                            &self.tags,
                                        )
                                    })
                                })
                            })
                    })
                    .unwrap_or_else(|| param_kind_surface_for_hover(&parameter.kind));

            if self
                .span_table
                .get(parameter.name_span)
                .contains(byte_offset)
            {
                return Some((word, surface));
            }

            // Keep declaration-level parameter hovers in signatures; typed
            // body-scope hovers are handled by typed-bind fallback for the
            // current declaration set.
            continue;
        }
        for typed_bind in self.defs.values() {
            let has_local_def = self.eval_ast.defs.values().any(|ast_bind| {
                ast_bind.name == typed_bind.name
                    || ast_bind.name.split('.').next_back() == Some(typed_bind.name.as_str())
            });
            if has_local_def {
                continue;
            }
            let Some((index, parameter_name)) = typed_bind
                .params
                .iter()
                .enumerate()
                .find_map(|(index, (name, _))| (name == &word).then_some((index, name)))
            else {
                continue;
            };
            let surface =
                signature_param_surface(&typed_bind.signature_surface, parameter_name.as_str())
                    .or_else(|| {
                        typed_bind.params.get(index).map(|(_, ty)| {
                            type_annotation_surface_for_hover_local(ty, &self.tag_types, &self.tags)
                        })
                    })
                    .unwrap_or_else(|| parameter_name.as_str().to_string());
            let body_contains = match &typed_bind.body {
                BindBody::Expr(expr) => self
                    .span_table
                    .get(self.exprs.span[expr.as_usize()])
                    .contains(byte_offset),
                BindBody::Body { exprs, ret } => {
                    exprs.iter().any(|expr| {
                        self.span_table
                            .get(self.exprs.span[expr.as_usize()])
                            .contains(byte_offset)
                    }) || ret.as_ref().is_some_and(|ret| {
                        self.span_table
                            .get(self.exprs.span[ret.as_usize()])
                            .contains(byte_offset)
                    })
                }
                BindBody::Extern => false,
            };
            let signature_contains = match &typed_bind.body {
                BindBody::Expr(expr) => {
                    byte_offset
                        < self
                            .span_table
                            .get(self.exprs.span[expr.as_usize()])
                            .start()
                }
                BindBody::Body { exprs, .. } => exprs.first().is_none_or(|expr| {
                    byte_offset
                        < self
                            .span_table
                            .get(self.exprs.span[expr.as_usize()])
                            .start()
                }),
                BindBody::Extern => false,
            };
            let this_def_start = self.span_table.get(typed_bind.name_span).start();
            if byte_offset < this_def_start {
                continue;
            }
            let next_def_start = def_starts
                .iter()
                .find(|start| **start > this_def_start)
                .copied()
                .unwrap_or(usize::MAX);
            if byte_offset >= next_def_start {
                continue;
            }
            if signature_contains || (body_contains && byte_offset < next_def_start) {
                return Some((word, surface));
            }
        }
        None
    }

    pub fn new(file_id: FileId, span_table: SpanTable) -> Self {
        Self {
            span_table,
            file_id,
            type_registry: crate::TypeRegistry::default(),
            target_layout: None,
            tags: HashMap::new(),
            defs: HashMap::new(),
            private_tags: HashSet::new(),
            private_defs: HashSet::new(),
            exprs: TypedExprVec::new(),
            root_exprs: Vec::new(),
            places: Vec::new(),
            place_versions: Vec::new(),
            place_version_components: Vec::new(),

            span_to_expr: BTreeMap::new(),
            tag_types: HashMap::new(),
            fn_return_types: HashMap::new(),
            variant_map: HashMap::new(),
            variant_annotations: HashMap::new(),
            imported_trait_names: HashSet::new(),
            self_contexts: Vec::new(),
            eval_ast: std::sync::Arc::new(ast::FileAst::empty_for_tests()),
            semantic_origin: None,
            module_doc: None,
            warnings: Vec::new(),
            declaration_flaws: Vec::new(),
            import_mod_paths: Vec::new(),
            parse_warnings: Vec::new(),
        }
    }

    pub fn tag(&self, id: &TagId) -> Option<&TypedTag> {
        self.tags.get(id)
    }

    pub fn def(&self, id: &DefId) -> Option<&TypedBind> {
        self.defs.get(id)
    }

    pub fn expr(&self, id: ExprId) -> Option<TypedExprRef<'_>> {
        self.exprs.get(id.as_usize())
    }

    pub fn fn_return_type(&self, id: &DefId) -> Option<&Ty> {
        self.fn_return_types.get(id)
    }

    pub fn tag_type(&self, id: &TagId) -> Option<&Ty> {
        self.tag_types.get(id)
    }

    pub fn lookup_variant(&self, name: &Intern<String>) -> Option<&Vec<VariantMapEntry>> {
        self.variant_map.get(name)
    }

    pub fn expr_at_byte(&self, byte_offset: u32) -> Option<ExprId> {
        let pos = byte_offset as usize;
        let mut best: Option<(usize, ExprId)> = None;
        for (i, span_id) in self.exprs.span.iter().enumerate() {
            if !span_id.is_valid() {
                continue;
            }
            let span = self.span_table.get(*span_id);
            if span.contains(pos) && best.is_none_or(|(best_len, _)| span.len() < best_len) {
                best = Some((span.len(), ExprId(i as u32)));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Tag whose declare site contains `byte_offset` (smallest span wins).
    ///
    /// Skips when the cursor is on a definition name or on another tag's name.
    /// Used for hovers on declare RHS (`in 0...255`, union variants, etc.).
    fn tag_at_byte(&self, byte_offset: usize, word: &str) -> Option<(&TagId, &TypedTag)> {
        if self.defs.values().any(|b| {
            b.name.as_str() == word && self.span_table.get(b.name_span).contains(byte_offset)
        }) {
            return None;
        }

        let mut best: Option<(&TagId, &TypedTag, usize)> = None;
        for (id, tag) in &self.tags {
            if !tag.span.is_valid() {
                continue;
            }
            let full = self.span_table.get(tag.span);
            if !full.contains(byte_offset) {
                continue;
            }
            let name = self.span_table.get(tag.name_span);
            if name.contains(byte_offset) && id.0.as_str() != word {
                continue;
            }
            if self.tags.iter().any(|(other, t)| {
                other != id
                    && other.0.as_str() == word
                    && self.span_table.get(t.name_span).contains(byte_offset)
            }) {
                continue;
            }
            let len = full.len();
            if best.is_none_or(|(_, _, bl)| len < bl) {
                best = Some((id, tag, len));
            }
        }
        best.map(|(id, tag, _)| (id, tag))
    }

    pub fn expr_at_source_pos(&self, source: &str, line: u32, character: u32) -> Option<ExprId> {
        let byte_offset = source.position_to_byte_offset(line, character)?;
        self.expr_at_byte(byte_offset as u32)
    }

    /// Check if the cursor is on a non-final segment of a qualified module path
    /// in any import statement. Returns the qualified module path (e.g. `"core.maybe"`).
    fn module_path_at_byte(&self, byte_offset: usize) -> Option<String> {
        for path in &self.import_mod_paths {
            let mp = &path.value;
            // Check root span (when there are segments)
            if !mp.segments.is_empty() && self.span_table.get(mp.root_span).contains(byte_offset) {
                return Some(mp.root.to_string());
            }
            // Check each segment span (non-final segments only)
            for (i, seg_span) in mp.segment_spans.iter().enumerate() {
                if i < mp.segments.len() - 1 && self.span_table.get(*seg_span).contains(byte_offset)
                {
                    let mut module = mp.root.to_string();
                    for j in 0..=i {
                        module.push('.');
                        module.push_str(mp.segments[j].as_str());
                    }
                    return Some(module);
                }
            }
        }
        None
    }

    /// Hover markdown for a resolved tag declaration.
    ///
    /// TODO: Add a "traits" section after the declaration text to show which
    /// interfaces/traits this type implements, e.g.:
    ///
    ///   ```gin
    ///   Bool is True or False
    ///   ```
    ///   ---
    ///   has [Happy](...), [ToString](...)
    ///   ---
    ///   Bool represents a value...
    ///
    /// The data is already available on `tag.provided_traits` (each `ProvidedTrait`
    /// has `trait_name` and `fields`. Use `tag.declaration_text` for the type
    /// signature block, then insert a `HoverSection::Prose` (or a new section variant)
    /// that lists trait names and links to their declarations. Also consider:
    ///
    /// - Auto-synthesized traits (e.g. `Reflectable` via `synthesize_reflectable_trait`)
    ///   — include them but perhaps distinguish from user-written `and has` clauses.
    /// - Auto trait defaults that apply to this type.
    fn hover_for_tag(&self, tag: &TypedTag) -> String {
        ast::hover_format::HoverDoc::new()
            .gin(&tag.declaration_text)
            .doc_opt(tag.doc_comment.as_ref())
            .render()
    }

    /// Hover for a union / const-union variant name.
    fn union_name_for_variant(
        &self,
        variant_map: &VariantMap,
        variant_name: &str,
    ) -> Option<Intern<String>> {
        variant_map
            .get(&Intern::<String>::from_ref(variant_name))
            .and_then(|entries| entries.first().map(|(union, _, _)| *union))
    }

    fn variant_pattern_label(
        &self,
        variant_map: &VariantMap,
        union_name: &str,
        variant_name: &str,
        tag_types: &HashMap<Intern<String>, Ty>,
        tag_params: Option<&HashMap<Intern<String>, Parameters>>,
    ) -> String {
        let key = Intern::<String>::from_ref(variant_name);
        if let Some(entries) = variant_map.get(&key) {
            let union_key = Intern::<String>::from_ref(union_name);
            if let Some((_, _, fields)) = entries.iter().find(|(u, _, _)| *u == union_key) {
                return format_variant_pattern_label(variant_name, fields, tag_types, tag_params);
            }
            if let Some((_, _, fields)) = entries.first() {
                return format_variant_pattern_label(variant_name, fields, tag_types, tag_params);
            }
        }
        variant_name.to_string()
    }

    fn hover_for_variant(&self, union_name: &str, variant_label: &str, _union_ty: &Ty) -> String {
        // Use just the base variant name (before any `(`) for annotation lookup.
        let base_name = variant_label.split('(').next().unwrap_or(variant_label);
        let annotation_key = format!("{union_name}.{base_name}");
        let doc_text = self
            .variant_annotations
            .get(&annotation_key)
            .and_then(|(_, doc)| doc.clone());
        let mut h = ast::hover_format::HoverDoc::new().gin(variant_label);
        if let Some(ref text) = doc_text {
            h = h.prose(text);
        }
        h.render()
    }

    fn variant_pattern_label_local(
        &self,
        variant_map: &VariantMap,
        union_name: &str,
        variant_name: &str,
    ) -> String {
        let key = Intern::<String>::from_ref(variant_name);
        if let Some(entries) = variant_map.get(&key) {
            let union_key = Intern::<String>::from_ref(union_name);
            if let Some((_, _, fields)) = entries.iter().find(|(u, _, _)| *u == union_key) {
                return format_variant_pattern_label_local(
                    variant_name,
                    fields,
                    &self.tag_types,
                    &self.tags,
                );
            }
            if let Some((_, _, fields)) = entries.first() {
                return format_variant_pattern_label_local(
                    variant_name,
                    fields,
                    &self.tag_types,
                    &self.tags,
                );
            }
        }
        variant_name.to_string()
    }

    fn span_contains(&self, span_id: SpanId, byte_offset: usize) -> bool {
        self.span_table.contains(span_id, byte_offset)
    }

    /// When the subject is an opaque tag name (e.g. `Type` before cross-file union resolve), use `tag_types`.
    fn resolve_pattern_subject_ty(&self, ty: &Ty, package: Option<&PackageSemanticIndex>) -> Ty {
        if let Ty::Opaque(name) = ty
            && let Some(resolved) = package
                .and_then(|p| p.tag_types.get(name))
                .or_else(|| self.tag_types.get(&TagId(*name)))
            && !matches!(resolved, Ty::Opaque(_))
        {
            return resolved.clone();
        }
        ty.clone()
    }

    fn hover_for_union_variant_word(
        &self,
        word: &str,
        variant_map: &VariantMap,
        package: Option<&PackageSemanticIndex>,
    ) -> Option<HoverResult> {
        let key = Intern::<String>::from_ref(word);
        let entries = variant_map.get(&key)?;
        let (union_name, _, _) = entries.first()?;
        let union_ty = package
            .and_then(|p| p.tag_types.get(union_name))
            .cloned()
            .or_else(|| self.tag_types.get(&TagId(*union_name)).cloned())
            .unwrap_or(Ty::Opaque(*union_name));
        let label = if let Some(p) = package {
            self.variant_pattern_label(
                variant_map,
                union_name.as_str(),
                word,
                &p.tag_types,
                Some(&p.tag_params),
            )
        } else {
            self.variant_pattern_label_local(variant_map, union_name.as_str(), word)
        };
        Some(
            HoverResult::single(self.hover_for_variant(union_name.as_str(), &label, &union_ty))
                .with_qualified_union(package, union_name),
        )
    }

    fn hover_pattern(&self, pattern: &Pattern, ctx: HoverPatternCtx<'_>) -> Option<HoverResult> {
        self.hover_pattern_node(pattern, ctx)
    }

    fn hover_pattern_node(
        &self,
        pattern: &Pattern,
        ctx: HoverPatternCtx<'_>,
    ) -> Option<HoverResult> {
        match pattern {
            Pattern::Generic {
                name,
                params,
                param_spans,
                span,
                ..
            } => {
                let head_span = self.span_table.get(*span);
                if head_span.start() <= ctx.byte_offset
                    && ctx.byte_offset < head_span.end()
                    && ctx.word == name.as_str()
                {
                    let union_key = match ctx.subject_ty {
                        Some(Ty::Union { name, .. }) => Some(*name),
                        _ => self.union_name_for_variant(ctx.variant_map, name.as_str()),
                    };
                    if let Some(union_key) = union_key {
                        let union_name = union_key.as_str();
                        let label = self.variant_pattern_label(
                            ctx.variant_map,
                            union_name,
                            name.as_str(),
                            ctx.tag_types,
                            ctx.tag_params,
                        );
                        let union_ty = ctx
                            .subject_ty
                            .cloned()
                            .or_else(|| {
                                ctx.package
                                    .and_then(|p| p.tag_types.get(&union_key))
                                    .cloned()
                                    .or_else(|| self.tag_types.get(&TagId(union_key)).cloned())
                            })
                            .unwrap_or(Ty::Opaque(union_key));
                        return Some(
                            HoverResult::single(
                                self.hover_for_variant(union_name, &label, &union_ty),
                            )
                            .with_qualified_union(ctx.package, &union_key),
                        );
                    }
                    return Some(HoverResult::single(format!("```gin\n{name}\n```")));
                }
                for (slot, (pname, pspan)) in param_spans.iter().enumerate() {
                    let ps = self.span_table.get(*pspan);
                    if ps.start() <= ctx.byte_offset
                        && ctx.byte_offset < ps.end()
                        && (ctx.word == pname.as_str()
                            || (pname.is_pattern_wildcard() && ctx.word == "_"))
                    {
                        if let Some(kind) = params.get(slot).map(|(_, k)| k)
                            && let ParameterKind::Tagged(sp) = kind
                            && sp.value.denotes_variant_name(ctx.word)
                            && let Some(h) = self.hover_for_union_variant_word(
                                ctx.word,
                                ctx.variant_map,
                                ctx.package,
                            )
                        {
                            return Some(h);
                        }
                        let ty_str = pattern
                            .pattern_param_type_at_slot(
                                slot,
                                ctx.subject_ty,
                                ctx.variant_map,
                                ctx.tag_types,
                            )
                            .map(|t| {
                                type_annotation_surface_for_hover(&t, ctx.tag_types, ctx.tag_params)
                            })
                            .unwrap_or_else(|| "infer".to_string());
                        let summary = if pname.is_pattern_wildcard() {
                            format!("_: `{ty_str}`")
                        } else {
                            format!("{} {}", ctx.word, ty_str)
                        };
                        return Some(HoverResult::single(format!("```gin\n{summary}\n```")));
                    }
                }
            }
            Pattern::ListCons { head, tail } => {
                let elem_ty = ctx.subject_ty.and_then(|ty| ty.list_elem_ty(ctx.tag_types));
                let head_ctx = HoverPatternCtx {
                    subject_ty: elem_ty.as_ref(),
                    ..ctx
                };
                if let Some(h) = self.hover_pattern_node(&head.value, head_ctx) {
                    return Some(h);
                }
                return self.hover_pattern_node(&tail.value, ctx);
            }
            Pattern::Tuple(elems) => {
                for e in elems {
                    if let Some(h) = self.hover_pattern_node(&e.value, ctx) {
                        return Some(h);
                    }
                }
            }
            Pattern::Nominal(name, span)
                if !name.is_pattern_wildcard()
                    && name
                        .as_str()
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_lowercase()) =>
            {
                let name_span = self.span_table.get(*span);
                if name_span.start() <= ctx.byte_offset
                    && ctx.byte_offset < name_span.end()
                    && ctx.word == name.as_str()
                {
                    let list_type_param = ctx.subject_ty.and_then(|subject_ty| match subject_ty {
                        Ty::Record { name, .. } if name.as_str() == "List" => {
                            ctx.tag_params.and_then(|params| {
                                params
                                    .get(&Intern::from_ref("List"))
                                    .and_then(|params| params.first())
                                    .map(|(name, _)| name.as_str().to_string())
                            })
                        }
                        _ => None,
                    });

                    let ty_str = ctx
                        .subject_ty
                        .map(|t| {
                            type_annotation_surface_for_hover(t, ctx.tag_types, ctx.tag_params)
                        })
                        .map(|surface| {
                            if name_span.start() <= ctx.byte_offset
                                && ctx.byte_offset < name_span.end()
                                && ctx.word == name.as_str()
                                && let Some(list_param) = list_type_param.as_deref()
                                && surface.starts_with("List(")
                            {
                                format!("List({list_param})")
                            } else {
                                surface
                            }
                        })
                        .unwrap_or_else(|| "infer".to_string());
                    let summary = format!("{} {}", ctx.word, ty_str);
                    return Some(HoverResult::single(format!("```gin\n{summary}\n```")));
                }
            }
            _ => {}
        }
        None
    }

    fn hover_type_pattern_at(
        &self,
        byte_offset: usize,
        word: &str,
        package: Option<&PackageSemanticIndex>,
    ) -> Option<HoverResult> {
        let owned_tag_types;
        let owned_tag_params;
        let (tag_types, tag_params) = if let Some(p) = package {
            (&p.tag_types, Some(&p.tag_params))
        } else {
            owned_tag_types = self
                .tag_types
                .iter()
                .map(|(id, ty)| (id.0, ty.clone()))
                .collect();
            owned_tag_params = self
                .tags
                .iter()
                .filter_map(|(id, tag)| tag.params.as_ref().map(|p| (id.0, p.clone())))
                .collect();
            (&owned_tag_types, Some(&owned_tag_params))
        };
        let variant_map = package.map(|p| &p.variant_map).unwrap_or(&self.variant_map);
        let pattern_ctx = HoverPatternCtx {
            byte_offset,
            word,
            subject_ty: None,
            tag_types,
            tag_params,
            variant_map,
            package,
        };
        for i in 0..self.exprs.kind.len() {
            match &self.exprs.kind[i] {
                TypedExprKind::When(w) => {
                    let subject_ty = w.subject.as_ref().map(|id| {
                        self.resolve_pattern_subject_ty(&self.exprs.ty[id.as_usize()], package)
                    });
                    let subject_ty = subject_ty.as_ref();
                    let pattern_ctx = HoverPatternCtx {
                        subject_ty,
                        ..pattern_ctx
                    };
                    for arm in &w.arms {
                        if let TypedWhenArm::Is { pattern, .. } = arm
                            && self.span_contains(pattern.span_id, byte_offset)
                            && let Some(h) = self.hover_pattern(&pattern.value, pattern_ctx)
                        {
                            return Some(h);
                        }
                    }
                }
                TypedExprKind::If(if_expr) => {
                    let mut patterns = Vec::new();
                    if_expr.condition.pattern_subjects(&mut patterns);
                    for (subject, pattern) in patterns {
                        if !self.span_contains(pattern.span_id, byte_offset) {
                            continue;
                        }
                        let subject_ty = Some(self.resolve_pattern_subject_ty(
                            &self.exprs.ty[subject.as_usize()],
                            package,
                        ));
                        let subject_ty = subject_ty.as_ref();
                        let pattern_ctx = HoverPatternCtx {
                            subject_ty,
                            ..pattern_ctx
                        };
                        if let Some(h) = self.hover_pattern(&pattern.value, pattern_ctx) {
                            return Some(h);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn param_surface_in_body(&self, byte_offset: usize, word: &str) -> Option<String> {
        let word = Intern::<String>::from_ref(word);
        self.eval_ast.defs.values().find_map(|bind| {
            if !self.bind_body_contains(bind, byte_offset) {
                return None;
            }
            bind.params
                .as_ref()?
                .get(&word)
                .map(|parameter| param_kind_surface_for_hover(&parameter.kind))
        })
    }

    fn bind_body_contains(&self, bind: &ast::Bind, byte_offset: usize) -> bool {
        match &bind.value {
            ast::BindValue::Expr(expr) => self.span_table.get(expr.span_id).contains(byte_offset),
            ast::BindValue::Body { exprs, ret } => {
                exprs
                    .iter()
                    .any(|expr| self.span_table.get(expr.span_id).contains(byte_offset))
                    || self.span_table.get(ret.span_id).contains(byte_offset)
            }
            ast::BindValue::Extern | ast::BindValue::Unassigned => false,
        }
    }

    /// Hover for a definition name (`arch` in `arch Architecture`, or a function def).
    fn parent_bind_name_for_body(&self, body: ExprId) -> Option<Intern<String>> {
        self.exprs.kind.iter().find_map(|kind| match kind {
            TypedExprKind::Bind {
                name,
                body: bind_body,
                unassigned: false,
                ..
            } if *bind_body == body => Some(*name),
            _ => None,
        })
    }

    fn hover_for_def(&self, bind: &TypedBind) -> String {
        ast::hover_format::HoverDoc::new()
            .gin(self.hover_signature_with_const(bind))
            .doc_opt(bind.doc_comment.as_ref())
            .render()
    }

    /// Signature surface plus the const RHS (e.g. `false Bool.False`) when the
    /// bind has no written type and the body folded to a constant.
    fn hover_signature_with_const(&self, bind: &TypedBind) -> String {
        let mut sig = bind.signature_surface.clone();
        if sig != bind.name.as_str() {
            return sig;
        }
        if bind.unassigned_decl {
            return sig;
        }
        let BindBody::Expr(expr_id) = &bind.body else {
            return sig;
        };
        let Some(expr) = self.expr(*expr_id) else {
            return sig;
        };
        if matches!(
            self.type_registry.resolved_definition_for_type(expr.ty),
            Ty::Record { .. }
        ) {
            return sig;
        }
        let Some(const_val) = expr.const_value else {
            return sig;
        };
        sig.push(' ');
        if let ConstValue::Tag {
            name: variant,
            qual_path: None,
            ..
        } = const_val
            && let Some(entries) = self.variant_map.get(variant)
            && let [(union, _, _)] = entries.as_slice()
        {
            sig.push_str(union.as_str());
            sig.push('.');
        }
        sig.push_str(&const_val.to_hover_string());
        sig
    }

    /// Get hover type information at a source position.
    /// Returns a string describing the type at that position, if available.
    pub fn hover_at(&self, source: &str, line: u32, character: u32) -> Option<String> {
        self.hover_at_with_package(source, line, character, None)
            .map(|r| r.markdown)
    }

    /// Like [`hover_at`](Self::hover_at) but uses a pre-built [`PackageSemanticIndex`] (no AST clone).
    /// Classify the thing at the cursor position into a [`HoverTarget`].
    #[doc(hidden)]
    pub fn classify_hover(
        &self,
        _source: &str,
        byte_offset: usize,
        word: &str,
        package: Option<&PackageSemanticIndex>,
    ) -> Option<HoverTarget> {
        let word_interned = Intern::<String>::from_ref(word);
        let tag_id = TagId(word_interned);

        // 0. Cursor is on `self` keyword — show the actual type (`self Type`,
        //    `ref self Type`, `mut self Type`) using the tag's resolved type.
        //    Must run before param_at_byte since `self` may also appear as a parameter.
        if word == "self"
            && let Some((tag_name, modifier)) = self.has_self_context_at_byte(byte_offset)
        {
            return Some(HoverTarget::SelfRef { modifier, tag_name });
        }

        // 1. Cursor is exactly on a definition name.
        if self.defs.values().any(|b| {
            b.name.as_str() == word && self.span_table.get(b.name_span).contains(byte_offset)
        }) {
            return Some(HoverTarget::Definition(TagId(word_interned).0));
        }

        // 1a. Cursor is on a function parameter declaration or a use in that function body.
        if let Some((name, surface)) = self.param_at_byte(byte_offset, word) {
            return Some(HoverTarget::Param { name, surface });
        }

        // 1b. Cursor is on a type parameter (e.g. `x` in `Range(x) has start x`.
        if let Some((_, tag)) = self.tag_at_byte(byte_offset, word)
            && let Some(params) = &tag.params
            && params.contains_key(&Intern::<String>::from_ref(word))
        {
            return Some(HoverTarget::Param {
                name: Intern::<String>::from_ref(word),
                surface: "type parameter".to_string(),
            });
        }

        // 2. Cursor is exactly on a tag declaration name.
        if let Some(tag) = self.tags.get(&tag_id) {
            if self.span_table.get(tag.name_span).contains(byte_offset) {
                return Some(HoverTarget::TagDecl(tag_id.0));
            }
            // 2a. Tag name referenced in an expression (e.g. `Bool` in `Bool.False`
            //     or a type annotation). Route to tag declaration hover.
            if let Some(expr_id) = self.expr_at_byte(byte_offset as u32)
                && let Some(expr_ref) = self.expr(expr_id)
            {
                let is_tag_ref = match &expr_ref.kind {
                    TypedExprKind::TagCall { variant_id, .. } => {
                        variant_id.union.0 == word_interned
                    }
                    _ => false,
                };
                if is_tag_ref {
                    return Some(HoverTarget::TagDecl(tag_id.0));
                }
            }
        } else if self.imported_trait_names.contains(&word_interned)
            && let Some(pkg) = package
            && pkg.tag_decl_for_word(&word_interned).is_some()
        {
            return Some(HoverTarget::TagDecl(tag_id.0));
        }

        // Literal expressions (like `'x86_64'`) can share their spelling with
        // literal-union variant names. Check if this word matches a variant before
        // falling through to the generic Expr classification.
        // Skip if the word is also a top-level definition — def hover wins or
        // the expression classification (step 6) handles it more informatively.
        let variant_map = package.map(|p| &p.variant_map).unwrap_or(&self.variant_map);
        if !self.defs.contains_key(&DefId(word_interned))
            && let Some(candidates) = variant_map.get(&word_interned)
            && let Some((union_name, discriminant, _)) = candidates.first()
        {
            return Some(HoverTarget::Variant {
                union_name: *union_name,
                discriminant: *discriminant,
                union_ty: Ty::Opaque(*union_name),
            });
        }

        // 3. Cursor is on a union variant name (checked before tag body
        //    so variants in union declarations get the variant-specific hover).
        //    Skip if the word is also a top-level definition — def hover wins.
        let variant_map = package.map(|p| &p.variant_map).unwrap_or(&self.variant_map);
        if !self.defs.contains_key(&DefId(word_interned))
            && let Some(candidates) = variant_map.get(&word_interned)
            && let Some((union_name, discriminant, _)) = candidates.first()
        {
            return Some(HoverTarget::Variant {
                union_name: *union_name,
                discriminant: *discriminant,
                union_ty: Ty::Opaque(*union_name),
            });
        }

        // 4. Word is a known tag name referenced inside a type annotation or tag
        //    body (e.g. `Bool` in `has b Bool` or `Pointer` in `pointer Pointer(x)`).
        //    Route to tag declaration directly — checked before `tag_at_byte` so
        //    it wins over the containing tag's body hover.
        if self.tags.contains_key(&tag_id) {
            return Some(HoverTarget::TagDecl(tag_id.0));
        }
        if self.imported_trait_names.contains(&word_interned)
            && let Some(pkg) = package
            && pkg.tag_decl_for_word(&word_interned).is_some()
        {
            return Some(HoverTarget::TagDecl(tag_id.0));
        }

        if word.starts_with(char::is_uppercase)
            && !self.tags.contains_key(&tag_id)
            && !self.imported_trait_names.contains(&word_interned)
        {
            return None;
        }

        // 5. Cursor is inside a tag declaration body (not the name or a variant).
        if let Some((tag_id, _)) = self.tag_at_byte(byte_offset, word) {
            return Some(HoverTarget::TagAtByte(tag_id.0));
        }

        // 6. Cursor is on a type pattern in a when/is arm.
        if let Some(h) = self.hover_type_pattern_at(byte_offset, word, package) {
            return Some(HoverTarget::TypePattern(h));
        }

        // 7. Cursor is on a non-final segment of a qualified module path (import).
        if let Some(module) = self.module_path_at_byte(byte_offset) {
            return Some(HoverTarget::ModulePath(module));
        }

        if let Some(surface) = self.record_field_surface_at_byte(byte_offset, word) {
            return Some(HoverTarget::RecordField {
                name: word_interned,
                surface,
            });
        }

        // 8. Cursor is on an expression.
        if let Some(expr_id) = self.expr_at_byte(byte_offset as u32) {
            return Some(HoverTarget::Expr(expr_id));
        }

        None
    }

    pub fn hover_at_with_package(
        &self,
        source: &str,
        line: u32,
        character: u32,
        package: Option<&PackageSemanticIndex>,
    ) -> Option<HoverResult> {
        let byte_offset = source.position_to_byte_offset(line, character)?;
        let word = source
            .symbol_at_byte_offset(byte_offset)
            .or_else(|| source.word_at_byte_offset(byte_offset))?;

        let target = self.classify_hover(source, byte_offset, &word, package)?;

        self.render_hover(source, target, &word, byte_offset, package)
    }

    /// Render markdown for a classified [`HoverTarget`].
    fn render_hover(
        &self,
        _source: &str,
        target: HoverTarget,
        word: &str,
        byte_offset: usize,
        package: Option<&PackageSemanticIndex>,
    ) -> Option<HoverResult> {
        match target {
            HoverTarget::Definition(name) => {
                let bind = self.defs.get(&DefId(name))?;
                let result = HoverResult::single(self.hover_for_def(bind));
                Some(result.with_def_module(package, &name))
            }
            HoverTarget::SelfRef { modifier, tag_name } => {
                // Resolve the receiver type from the tag declaration.
                let ty = self.tags.get(&TagId(tag_name)).map(|tag| &tag.resolved_ty);
                if let Some(ty) = ty {
                    let ty_str = if let Some(p) = package {
                        type_annotation_surface_for_hover(ty, &p.tag_types, Some(&p.tag_params))
                    } else {
                        type_annotation_surface_for_hover_local(ty, &self.tag_types, &self.tags)
                    };
                    let rendered = if modifier.as_str() == "self" {
                        format!("self {ty_str}")
                    } else {
                        format!("{} {ty_str}", modifier.as_str())
                    };
                    Some(HoverResult::single(
                        ast::hover_format::HoverDoc::new().gin(rendered).render(),
                    ))
                } else {
                    // Tag not found locally — check package index for cross-file tags.
                    let ty_str = package.and_then(|p| {
                        p.tag_types.get(&tag_name).map(|ty| {
                            type_annotation_surface_for_hover(ty, &p.tag_types, Some(&p.tag_params))
                        })
                    });
                    if let Some(ty_str) = ty_str {
                        let rendered = if modifier.as_str() == "self" {
                            format!("self {ty_str}")
                        } else {
                            format!("{} {ty_str}", modifier.as_str())
                        };
                        Some(HoverResult::single(
                            ast::hover_format::HoverDoc::new().gin(rendered).render(),
                        ))
                    } else {
                        // Last resort: just show the tag name.
                        let rendered = if modifier.as_str() == "self" {
                            format!("self {tag_name}")
                        } else {
                            format!("{} {tag_name}", modifier.as_str())
                        };
                        Some(HoverResult::single(
                            ast::hover_format::HoverDoc::new().gin(rendered).render(),
                        ))
                    }
                }
            }
            HoverTarget::Param { name, surface } => Some(HoverResult::single(
                ast::hover_format::HoverDoc::new()
                    .gin(format!("{} {}", name.as_str(), surface))
                    .render(),
            )),
            HoverTarget::RecordField { name, surface } => Some(HoverResult::single(
                ast::hover_format::HoverDoc::new()
                    .gin(format!("{} {}", name.as_str(), surface))
                    .render(),
            )),
            HoverTarget::TagDecl(name) => {
                let tag_id = TagId(name);
                // Try local tags first, then fall back to cross-file package index.
                if let Some(tag) = self.tags.get(&tag_id) {
                    let result = HoverResult::single(self.hover_for_tag(tag));
                    return Some(result.with_tag_module(package, &tag_id.0));
                }
                if let Some(pkg) = package
                    && let Some((_module, tag)) = pkg.tag_decl_for_word(&name)
                {
                    let result = HoverResult::single(self.hover_for_tag(tag));
                    return Some(result.with_tag_module(package, &name));
                }
                None
            }
            HoverTarget::TagAtByte(name) => {
                let tag_id = TagId(name);
                let tag = self.tags.get(&tag_id)?;

                if let Some(decl) = self.eval_ast.tags.get(&name)
                    && let ast::DeclareValue::Has(members) = &decl.value
                {
                    for member in members {
                        match member {
                            ast::HasMember::Property(p)
                                if p.name.as_str() == word
                                    && self.span_table.get(p.name_span).contains(byte_offset) =>
                            {
                                let member_key = Intern::<String>::from_ref(word);
                                let ty_surface = tag
                                    .record_field_types
                                    .get(&member_key)
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        p.ty.as_ref()
                                            .map(|ty| ty.value.format_surface())
                                            .unwrap_or_default()
                                    });
                                let ty_surface = ty_surface.trim_start();
                                let shape = if ty_surface.starts_with('(') {
                                    format!("{word}{ty_surface}")
                                } else {
                                    format!("{word} {ty_surface}")
                                };
                                let mut hover = ast::hover_format::HoverDoc::new().gin(shape);
                                if let Some(doc) = tag.record_field_docs.get(&member_key) {
                                    hover = hover.prose(doc.clone());
                                }
                                let result = HoverResult::single(hover.render());
                                return Some(result.with_qualified_union(package, &tag_id.0));
                            }
                            ast::HasMember::Function(f)
                                if f.name.as_str() == word
                                    && self.span_table.get(f.name_span).contains(byte_offset) =>
                            {
                                let mut signature = String::new();
                                if !f.params.is_empty() {
                                    signature.push('(');
                                    let mut first = true;
                                    for (k, v) in f.params.iter() {
                                        if !first {
                                            signature.push_str(", ");
                                        }
                                        first = false;
                                        if let Some(conv) = f.conventions.get(k) {
                                            match conv {
                                                ast::ParamConvention::Observe => {
                                                    signature.push_str("ref ")
                                                }
                                                ast::ParamConvention::Mutate => {
                                                    signature.push_str("mut ")
                                                }
                                                ast::ParamConvention::Consume => {
                                                    signature.push_str("eat ")
                                                }
                                                ast::ParamConvention::Own => {}
                                            }
                                        }
                                        signature.push_str(k.as_str());
                                        match &v.kind {
                                            ast::ParameterKind::Tagged(sp) => {
                                                signature.push(' ');
                                                signature.push_str(&sp.value.format_surface());
                                            }
                                            ast::ParameterKind::ValueParam { ty } => {
                                                signature.push(' ');
                                                signature.push_str(&ty.value.format_surface());
                                            }
                                            ast::ParameterKind::Inferred { ty } => {
                                                signature.push(' ');
                                                signature.push_str(&ty.value.format_surface());
                                                signature.push_str(": ?");
                                            }
                                            ast::ParameterKind::Default(expr) => {
                                                use std::fmt::Write as _;
                                                let _ = write!(&mut signature, ": {:?}", expr);
                                            }
                                            ast::ParameterKind::Generic => {}
                                        }
                                    }
                                    signature.push(')');
                                }
                                if let Some(rt) = &f.return_ty {
                                    signature.push(' ');
                                    signature.push_str(&rt.value.format_surface());
                                }
                                if let Some(et) = &f.error_ty {
                                    signature.push_str(" or ");
                                    signature.push_str(&et.value.format_surface());
                                }
                                let rendered =
                                    format!("```gin\n{}{}\n```", f.name.as_str(), signature)
                                        .replacen(" (", "(", 1);
                                let rendered = rendered
                                    .strip_prefix("```gin\n")
                                    .and_then(|it| it.strip_suffix("\n```"))
                                    .unwrap_or(&rendered);
                                let mut hover = ast::hover_format::HoverDoc::new().gin(rendered);
                                if let Some(doc) = tag.record_field_docs.get(&f.name) {
                                    hover = hover.prose(doc.clone());
                                }
                                let result = HoverResult::single(hover.render());
                                return Some(result.with_qualified_union(package, &tag_id.0));
                            }
                            _ => {}
                        }
                    }
                }

                let member_key = Intern::<String>::from_ref(word);
                if let Some(ty_surface) = tag.record_field_types.get(&member_key) {
                    let shape = if ty_surface.starts_with('(') {
                        format!("{}{}", word, ty_surface)
                    } else {
                        format!("{} {}", word, ty_surface)
                    };
                    let mut hover = ast::hover_format::HoverDoc::new().gin(shape);
                    if let Some(doc) = tag.record_field_docs.get(&member_key) {
                        hover = hover.prose(doc.clone());
                    }
                    let result = HoverResult::single(hover.render());
                    return Some(result.with_qualified_union(package, &tag_id.0));
                }

                if let Ty::Record { fields, .. } = self
                    .type_registry
                    .resolved_definition_for_type(&tag.resolved_ty)
                    && let Some((_, fty)) = fields.iter().find(|(n, _)| *n == member_key)
                {
                    let ty_str = {
                        let display_ty: &Ty = match &**fty {
                            Ty::Ptr { inner } => inner.as_ref(),
                            Ty::Address { pointee, .. } => pointee.as_ref(),
                            other => other,
                        };
                        type_annotation_surface_for_hover_local(
                            display_ty,
                            &self.tag_types,
                            &self.tags,
                        )
                    };
                    let shape = if ty_str.starts_with('(') {
                        format!("{}{}", word, ty_str)
                    } else {
                        format!("{} {}", word, ty_str)
                    };
                    let member_doc = tag.record_field_docs.get(&member_key).map(|s| s.as_str());
                    let mut hover = ast::hover_format::HoverDoc::new().gin(shape);
                    if let Some(doc) = member_doc {
                        hover = hover.prose(doc.to_string());
                    }
                    let result = HoverResult::single(hover.render());
                    return Some(result.with_qualified_union(package, &tag_id.0));
                }
                let result = HoverResult::single(self.hover_for_tag(tag));
                Some(result.with_tag_module(package, &name))
            }
            HoverTarget::ModulePath(module) => {
                let doc = package
                    .and_then(|p| p.module_docs.get(&module))
                    .map(|d| ast::hover_format::HoverDoc::new().prose(d.clone()).render())
                    .unwrap_or_else(|| format!("`{module}`"));
                Some(HoverResult::single(doc))
            }
            HoverTarget::TypePattern(hover) => Some(hover),
            HoverTarget::Variant {
                union_name,
                discriminant,
                union_ty,
            } => {
                let resolved_ty = package
                    .and_then(|p| p.tag_types.get(&union_name))
                    .or_else(|| self.tag_types.get(&TagId(union_name)))
                    .unwrap_or(&union_ty);
                let resolved_definition =
                    self.type_registry.resolved_definition_for_type(resolved_ty);
                // For literal-constant unions (e.g. `Architecture is 'x86_64' or 'arm64'`),
                // show the parent tag declaration instead of just the literal label,
                // so the user sees the full union shape.
                if resolved_definition.union_literal_values().is_some() {
                    if let Some(tag) = self.tags.get(&TagId(union_name)) {
                        let result = HoverResult::single(self.hover_for_tag(tag));
                        return Some(result.with_tag_module(package, &union_name));
                    }
                    if let Some(pkg) = package
                        && let Some((_module, tag)) = pkg.tag_decl_for_word(&union_name)
                    {
                        let result = HoverResult::single(self.hover_for_tag(tag));
                        return Some(result.with_tag_module(package, &union_name));
                    }
                }
                let variant_label = const_union_variant_label(&resolved_definition, discriminant)
                    .unwrap_or_else(|| {
                        // Format variant with its fields (e.g. `Some(x)` not just `Some`)
                        let variant_map =
                            package.map(|p| &p.variant_map).unwrap_or(&self.variant_map);
                        let key = Intern::<String>::from_ref(word);
                        if let Some(entries) = variant_map.get(&key)
                            && let Some((_, _, fields)) = entries.first()
                        {
                            return if let Some(p) = package {
                                format_variant_pattern_label(word, fields, &p.tag_types, None)
                            } else {
                                format_variant_pattern_label_local(
                                    word,
                                    fields,
                                    &self.tag_types,
                                    &self.tags,
                                )
                            };
                        }
                        word.to_string()
                    });
                let result = HoverResult::single(self.hover_for_variant(
                    union_name.as_str(),
                    &variant_label,
                    &resolved_definition,
                ));
                Some(result.with_qualified_union(package, &union_name))
            }
            HoverTarget::Expr(expr_id) => {
                if let Some(surface) = self.param_surface_in_body(byte_offset, word) {
                    return Some(HoverResult::single(
                        ast::hover_format::HoverDoc::new()
                            .gin(format!("{word} {surface}"))
                            .render(),
                    ));
                }
                let expr_ref = self.expr(expr_id)?;
                if let TypedExprKind::FnCall { target, .. } = &expr_ref.kind
                    && word == target.0.as_str()
                    && let Some(bind) = self.defs.get(target)
                {
                    let result = HoverResult::single(self.hover_for_def(bind));
                    return Some(result.with_def_module(package, &target.0));
                }
                if matches!(expr_ref.kind, TypedExprKind::Lit(_))
                    && let Some(name) = self.parent_bind_name_for_body(expr_id)
                    && self
                        .type_registry
                        .resolved_definition_for_type(expr_ref.ty)
                        .union_literal_values()
                        .is_some()
                {
                    return Some(HoverResult::single(format!(
                        "{} union\n---\n\n",
                        name.as_str()
                    )));
                }
                let ty_str = if let Some(p) = package {
                    type_annotation_surface_for_hover(
                        expr_ref.ty,
                        &p.tag_types,
                        Some(&p.tag_params),
                    )
                } else {
                    type_annotation_surface_for_hover_local(
                        expr_ref.ty,
                        &self.tag_types,
                        &self.tags,
                    )
                };
                let summary = match &expr_ref.kind {
                    TypedExprKind::Bind {
                        name,
                        unassigned,
                        body,
                        ..
                    } => {
                        let no_explicit_type = !unassigned;
                        // Try bind's own const_value first; if None, try to find it
                        // from the body expression (local const binds store the literal
                        // value on the body expression, not on the bind itself).
                        let cv = expr_ref.const_value.as_ref().or_else(|| {
                            let body_idx = body.as_usize();
                            if body_idx < self.exprs.const_value.len()
                                && self.exprs.const_value[body_idx].is_some()
                            {
                                return self.exprs.const_value[body_idx].as_ref();
                            }
                            None
                        });
                        if no_explicit_type && let Some(cv) = cv {
                            // `:=` bind with const value, no explicit type: show `x 42`
                            format!("{} {}", name.as_str(), cv.to_hover_string())
                        } else {
                            // Explicit type annotation or no const: show `x Int`
                            format!("{} {}", name.as_str(), ty_str)
                        }
                    }
                    TypedExprKind::FnCall { target, .. } => {
                        if word == target.0.as_str() && !self.defs.contains_key(target) {
                            format!("{} {}", target.0.as_str(), ty_str)
                        } else {
                            format!("`{}`: `{ty_str}`", target.0.as_str())
                        }
                    }
                    TypedExprKind::TagCall { variant_id, .. } => {
                        if let Some(cv) = &expr_ref.const_value {
                            cv.to_hover_string()
                        } else {
                            format!(
                                "`{}` (variant of `{}`)",
                                variant_id.name.as_str(),
                                variant_id.union.0.as_str(),
                            )
                        }
                    }
                    TypedExprKind::Lit(_) => match &expr_ref.const_value {
                        Some(cv) => cv.to_hover_string(),
                        None => ty_str,
                    },
                    _ => ty_str,
                };
                let summary = self.integer_hover_summary(expr_id, summary);
                Some(HoverResult::single(
                    ast::hover_format::HoverDoc::new().inline(summary).render(),
                ))
            }
        }
    }

    fn integer_hover_summary(&self, expr_id: ExprId, summary: String) -> String {
        let ty = &self.exprs.ty[expr_id.as_usize()];
        let Some(validity) = self.type_registry.integer_validity_for_type(ty) else {
            return summary;
        };
        let identity = if ty.named_instance_stripping_reference_wrappers().is_some() {
            ty.format_for_hover()
        } else {
            "anonymous integer".to_string()
        };
        let knowledge = match self
            .exprs
            .integer_knowledge_of(expr_id)
            .and_then(Option::as_ref)
        {
            Some(ast::integer::IntegerKnowledge::Exact(
                ast::integer::CanonicalIntegerExpr::Value(value),
            )) => format!("exactly {value}"),
            Some(ast::integer::IntegerKnowledge::Exact(
                ast::integer::CanonicalIntegerExpr::Symbolic(value),
            )) => format!("exactly {value}"),
            Some(ast::integer::IntegerKnowledge::Domain(domain)) => domain
                .storage_hull()
                .map(|hull| format!("{}...{}", hull.min(), hull.max()))
                .unwrap_or_else(|| "symbolic domain".to_string()),
            Some(ast::integer::IntegerKnowledge::Unknown) | None => "unknown".to_string(),
            Some(ast::integer::IntegerKnowledge::Poison) => "poison".to_string(),
        };
        let validity = validity
            .domain()
            .storage_hull()
            .map(|hull| format!("{}...{}", hull.min(), hull.max()))
            .unwrap_or_else(|| "no finite hull".to_string());
        let representation = self
            .type_registry
            .integer_width_for_type(ty)
            .map(|width| format!("i{width}"))
            .unwrap_or_else(|| "unresolved".to_string());
        let interpretation = self
            .type_registry
            .nominal_integer_interpretation_for_type(ty)
            .map(|interpretation| format!("\ninterpretation: {interpretation:?}"))
            .unwrap_or_default();
        format!(
            "{summary}\n{identity}\ncurrent knowledge: {knowledge}\nvalidity: {validity}\nruntime representation: {representation}{interpretation}"
        )
    }

    fn record_field_surface_at_byte(&self, byte_offset: usize, word: &str) -> Option<String> {
        let field = Intern::<String>::from_ref(word);
        for (expr_index, kind) in self.exprs.kind.iter().enumerate() {
            let TypedExprKind::TupleGet { base, index } = kind else {
                continue;
            };
            if !self
                .span_table
                .get(self.exprs.span[expr_index])
                .contains(byte_offset)
            {
                continue;
            }
            let base_definition = self
                .exprs
                .ty
                .get(base.as_usize())
                .map(|ty| self.type_registry.resolved_definition_for_type(ty));
            if let Some(Ty::Record {
                name: tag_name,
                fields,
                ..
            }) = base_definition
                && let Some((name, ty)) = fields.get(*index)
                && *name == field
            {
                if let Some(tag) = self.tags.get(&TagId(tag_name))
                    && let Some(surface) = tag.record_field_types.get(&field)
                {
                    return Some(surface.clone());
                }
                return Some(format_ty_for_hover(ty));
            }
            let TypedExprKind::FnCall { target, .. } = self.exprs.kind.get(base.as_usize())? else {
                continue;
            };
            let bind = self.defs.get(target)?;
            let body = match &bind.body {
                BindBody::Expr(expr) => Some(*expr),
                BindBody::Body { exprs, ret } => (*ret).or_else(|| exprs.last().copied()),
                BindBody::Extern => None,
            }?;
            let TypedExprKind::TagCall { variant_id, .. } = self.exprs.kind.get(body.as_usize())?
            else {
                continue;
            };
            let tag = self.tags.get(&variant_id.union)?;
            let Ty::Record { fields, .. } = self
                .type_registry
                .resolved_definition_for_type(&tag.resolved_ty)
            else {
                continue;
            };
            let (name, _) = fields.get(*index)?;
            if *name == field {
                return tag.record_field_types.get(&field).cloned();
            }
            continue;
        }
        None
    }

    /// Resolve the type of a field access expression at a source position.
    /// For a position at `expr.field`, finds the expression before the dot,
    /// looks up its type, and if it's a Record, returns the field's type
    /// formatted via [`format_ty_for_hover`].
    pub fn dot_type(&self, source: &str, line: u32, character: u32) -> Option<String> {
        let byte_offset = source.position_to_byte_offset(line, character)?;

        // Check that there's a dot before the cursor position.
        let dot_pos = byte_offset.checked_sub(1)?;
        if source.as_bytes().get(dot_pos) != Some(&b'.') {
            return None;
        }

        // Extract the field name (the word at the cursor position).
        let field_name = source.word_at_byte_offset(byte_offset)?;

        // Find the expression whose span covers the dot position.
        let expr_id = self.expr_at_byte(dot_pos as u32)?;
        let expr_ref = self.expr(expr_id)?;

        // If the expression has a Record type, look up the field by name.
        match self.type_registry.resolved_definition_for_type(expr_ref.ty) {
            Ty::Record { fields, .. } => {
                let interned_field = Intern::<String>::from_ref(&field_name);
                for (name, ty) in fields {
                    if name == interned_field {
                        return Some(format_ty_for_hover(&ty));
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Get the definition span for a symbol at a position.
    /// Returns (start_byte, end_byte) of the definition.
    pub fn definition_span(
        &self,
        source: &str,
        line: u32,
        character: u32,
    ) -> Option<(usize, usize)> {
        let expr_id = self.expr_at_source_pos(source, line, character)?;
        let expr_ref = self.expr(expr_id)?;

        match expr_ref.kind {
            TypedExprKind::Bind { name, .. } => {
                for bind in self.defs.values() {
                    if bind.name == *name {
                        let span = self.span_table.get(bind.name_span);
                        return Some((span.start(), span.end()));
                    }
                }
                None
            }
            TypedExprKind::FnCall { target, .. } => {
                if let Some(bind) = self.defs.get(target) {
                    let span = self.span_table.get(bind.name_span);
                    return Some((span.start(), span.end()));
                }
                None
            }
            TypedExprKind::TagCall { .. } => None,
            _ => None,
        }
    }

    /// Collect all type flaws from the expression arena and bind-level diagnostics.
    pub fn all_flaws(&self) -> Vec<(SpanId, &Diagnostic)> {
        let mut flaws = Vec::new();
        for i in 0..self.exprs.kind.len() {
            let span_id = self.exprs.span[i];
            for flaw in &self.exprs.flaws[i] {
                let flaw_span = if flaw.code.slug() == "type-unreachable-else-arm" {
                    if let TypedExprKind::When(when_expr) = &self.exprs.kind[i] {
                        when_expr
                            .arms
                            .iter()
                            .find_map(|arm| match arm {
                                TypedWhenArm::Else(_, sub) => Some(sub.into_inner()),
                                _ => None,
                            })
                            .unwrap_or(span_id)
                    } else {
                        span_id
                    }
                } else {
                    span_id
                };
                flaws.push((flaw_span, flaw));
            }
        }
        for bind in self.defs.values() {
            for flaw in &bind.flaws {
                flaws.push((bind.name_span, flaw));
            }
        }
        for (span_id, flaw) in &self.declaration_flaws {
            flaws.push((*span_id, flaw));
        }
        flaws
    }

    /// Collect declaration-level warnings.
    pub fn all_warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }
}

fn signature_param_surface(signature: &str, name: &str) -> Option<String> {
    let open = signature.find('(')?;
    let close = signature.rfind(')')?;
    let params = signature.get(open + 1..close)?;
    for part in params.split(',') {
        let part = part.trim();
        let rest = part.strip_prefix(name)?.trim_start();
        if rest.is_empty()
            || rest.starts_with(':')
            || rest.starts_with(|c: char| c.is_ascii_uppercase())
        {
            return Some(rest.to_string());
        }
    }
    None
}

fn param_kind_surface_for_hover(kind: &ParameterKind) -> String {
    match kind {
        ParameterKind::Tagged(sp) => sp.value.format_surface(),
        ParameterKind::ValueParam { ty } => ty.value.format_surface(),
        ParameterKind::Inferred { ty } => {
            format!("{}: ?", ty.value.format_surface())
        }
        ParameterKind::Generic => String::new(),
        ParameterKind::Default(expr) => format!(": {:?}", expr.value),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverResult {
    pub markdown: String,
    /// Qualified path to the base type (e.g. `core.primitive.Bool` for variant `False`).
    pub module_prefix: Option<String>,
}

impl HoverResult {
    pub fn single(markdown: String) -> Self {
        Self {
            markdown,
            module_prefix: None,
        }
    }

    /// Prefix hover with `{module}` when the tag's declaring module is known.
    pub fn with_tag_module(
        mut self,
        package: Option<&PackageSemanticIndex>,
        tag_name: &Intern<String>,
    ) -> Self {
        if self.module_prefix.is_none()
            && let Some(pkg) = package
            && let Some(module) = pkg.tag_module.get(tag_name)
        {
            self.module_prefix = Some(module.clone());
        }
        self
    }

    /// Prefix hover with `{module}.{def}` when the definition's declaring module is known.
    pub fn with_def_module(
        mut self,
        package: Option<&PackageSemanticIndex>,
        def_name: &Intern<String>,
    ) -> Self {
        if self.module_prefix.is_none()
            && let Some(pkg) = package
            && let Some(path) = pkg.qualified_def_path(def_name)
        {
            self.module_prefix = Some(path);
        }
        self
    }

    /// Prefix hover with `{module}.{union}` when the union's declaring module is known.
    pub fn with_qualified_union(
        mut self,
        package: Option<&PackageSemanticIndex>,
        union_name: &Intern<String>,
    ) -> Self {
        if self.module_prefix.is_none()
            && let Some(pkg) = package
            && let Some(path) = pkg.qualified_union_path(union_name)
        {
            self.module_prefix = Some(path);
        }
        self
    }
}

/// Cross-file tag and variant index for package-scoped IDE hover.
#[derive(Clone, PartialEq)]
pub struct PackageSemanticIndex {
    pub type_registry: crate::TypeRegistry,
    pub tag_types: HashMap<Intern<String>, Ty>,
    pub tag_params: HashMap<Intern<String>, Parameters>,
    pub variant_map: VariantMap,
    /// Declaring module per tag name (`Bool` → `core.primitive`).
    /// Qualified path to the base type (e.g. `core.primitive.Bool` for variant `False`).
    pub tag_module: HashMap<Intern<String>, String>,
    /// Tag declarations from across the package for cross-file name hovers.
    pub tag_decls: HashMap<Intern<String>, TypedTag>,
    /// Declaring module per definition name (`Default` → `core.default`).
    pub def_module: HashMap<Intern<String>, String>,
    /// Merged module-level doc comments (`--|`), keyed by qualified module path.
    /// Populated by [`stage_build_index`] — concatenates docs from all files in
    /// each module directory in alphabetical filename order.
    pub module_docs: HashMap<String, String>,
}

impl PackageSemanticIndex {
    pub fn tag_decl_for_word(&self, name: &Intern<String>) -> Option<(&String, &TypedTag)> {
        self.tag_module.get(name).zip(self.tag_decls.get(name))
    }
}

impl PackageSemanticIndex {
    pub fn from_typed_asts(asts: &[&TypedFileAst]) -> Self {
        let mut tag_types: HashMap<Intern<String>, Ty> = HashMap::new();
        let mut tag_params: HashMap<Intern<String>, Parameters> = HashMap::new();
        let mut type_registry = crate::TypeRegistry::default();
        let mut tag_module = HashMap::new();
        let mut tag_decls = HashMap::new();
        let mut def_module = HashMap::new();
        let mut module_docs: HashMap<String, Vec<String>> = HashMap::new();

        for ast in asts {
            type_registry.merge_from(&ast.type_registry);
            for (tag_id, ty) in &ast.tag_types {
                tag_types.insert(tag_id.0, ty.clone());
            }
            for (tag_id, tag) in &ast.tags {
                if let Some(params) = &tag.params {
                    tag_params.insert(tag_id.0, params.clone());
                }
            }
            if let Some(origin) = ast.semantic_origin.as_ref() {
                let simple_module = origin
                    .module
                    .iter()
                    .map(|part| part.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                let module = if simple_module.is_empty() {
                    origin.package.name.as_str().to_string()
                } else {
                    format!("{}.{}", origin.package.name.as_str(), simple_module)
                };
                for (tag_id, tag) in &ast.tags {
                    tag_module.entry(tag_id.0).or_insert_with(|| module.clone());
                    tag_decls.insert(tag_id.0, tag.clone());
                }
                for def_id in ast.defs.keys() {
                    def_module.entry(def_id.0).or_insert_with(|| module.clone());
                }
                if let Some(doc) = ast.module_doc.as_ref() {
                    module_docs
                        .entry(module)
                        .or_default()
                        .push(doc.value.clone());
                    if !simple_module.is_empty() {
                        module_docs
                            .entry(simple_module)
                            .or_default()
                            .push(doc.value.clone());
                    }
                }
            }
        }

        let variant_map = collect_package_variant_map(asts);

        Self {
            type_registry,
            tag_types,
            tag_params,
            variant_map,
            tag_module,
            tag_decls,
            def_module,
            module_docs: module_docs
                .into_iter()
                .map(|(module, docs)| (module, docs.join("\n\n")))
                .collect(),
        }
    }

    pub fn module_for_variant_word(&self, variant: &Intern<String>) -> Option<&String> {
        let (union, _, _) = self.variant_map.get(variant)?.first()?;
        self.tag_module.get(union)
    }

    /// `core.primitive.Bool` for union `Bool` declared in module `core.primitive`.
    pub fn qualified_union_path(&self, union: &Intern<String>) -> Option<String> {
        let module = self.tag_module.get(union)?;
        Some(format!("{module}.{}", union.as_str()))
    }

    /// `core.default` for definition `Default` declared in module `core.default`.
    pub fn qualified_def_path(&self, def: &Intern<String>) -> Option<String> {
        self.def_module.get(def).cloned()
    }
}

fn const_union_variant_label(ty: &Ty, discriminant: usize) -> Option<String> {
    let values = ty.union_literal_values()?;
    let cv = values.get(discriminant)?;
    match cv {
        ast::ConstValue::String(s) => Some(format!("'{s}'")),
        _ => Some(cv.to_hover_string()),
    }
}

fn format_variant_pattern_label(
    name: &str,
    fields: &[(Intern<String>, Ty)],
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> String {
    if fields.is_empty() {
        return name.to_string();
    }
    let parts: Vec<String> = fields
        .iter()
        .map(|(n, t)| {
            let type_str = variant_pattern_field_type_surface(t, tag_types, tag_params);
            if type_str == n.as_str() {
                // Opaque generic matching the field name — show just the name.
                n.as_str().to_string()
            } else {
                format!("{} {}", n.as_str(), type_str)
            }
        })
        .collect();
    format!("{}({})", name, parts.join(", "))
}

fn format_variant_pattern_label_local(
    name: &str,
    fields: &[(Intern<String>, Ty)],
    tag_types: &HashMap<TagId, Ty>,
    tags: &HashMap<TagId, TypedTag>,
) -> String {
    if fields.is_empty() {
        return name.to_string();
    }
    let parts: Vec<String> = fields
        .iter()
        .map(|(n, t)| {
            let type_str = variant_pattern_field_type_surface_local(t, tag_types, tags);
            if type_str == n.as_str() {
                n.as_str().to_string()
            } else {
                format!("{} {}", n.as_str(), type_str)
            }
        })
        .collect();
    format!("{}({})", name, parts.join(", "))
}

fn variant_pattern_field_type_surface(
    ty: &Ty,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> String {
    if let Ty::Record { name, .. } = ty
        && name.as_str() != "List"
    {
        return format_ty_for_hover(ty).replace("Union(Type)", "Union(union)");
    } else if let Ty::Record { name, .. } = ty
        && name.as_str() == "List"
    {
        return "List".to_string();
    }
    type_annotation_surface_for_hover(ty, tag_types, tag_params)
}

fn variant_pattern_field_type_surface_local(
    ty: &Ty,
    tag_types: &HashMap<TagId, Ty>,
    tags: &HashMap<TagId, TypedTag>,
) -> String {
    if let Ty::Record { name, .. } = ty
        && name.as_str() != "List"
    {
        return format_ty_for_hover(ty).replace("Union(Type)", "Union(union)");
    } else if let Ty::Record { name, .. } = ty
        && name.as_str() == "List"
    {
        return "List".to_string();
    }
    type_annotation_surface_for_hover_local(ty, tag_types, tags)
}

/// Type name as written in a `name Type` declare (e.g. `List(NamedTy)` not `pointer: …`).
pub(crate) fn type_annotation_surface_for_hover(
    ty: &Ty,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> String {
    match ty {
        Ty::Union { name, .. } => {
            if name.as_str() == "union"
                && let Some((tag_name, _)) = tag_types.iter().find(|(_, tag_ty)| *tag_ty == ty)
            {
                return tag_name.as_str().to_string();
            }
            name.as_str().to_string()
        }
        Ty::Opaque(name) => name.as_str().to_string(),
        Ty::Record {
            name,
            fields,
            resolved_params,
        } => {
            if let Some(surface) = generic_record_surface(
                name,
                fields,
                resolved_params.as_deref(),
                tag_types,
                tag_params,
            ) {
                return surface;
            }
            if tag_types.contains_key(name) && !tag_has_generic_params(name, tag_params) {
                return name.as_str().to_string();
            }
            format_ty_for_hover(ty)
        }
        other => format_ty_for_hover(other),
    }
}

pub(crate) fn type_annotation_surface_for_hover_local(
    ty: &Ty,
    tag_types: &HashMap<TagId, Ty>,
    tags: &HashMap<TagId, TypedTag>,
) -> String {
    match ty {
        Ty::Union { name, .. } => {
            if name.as_str() == "union"
                && let Some((tag_name, _)) = tag_types.iter().find(|(_, tag_ty)| *tag_ty == ty)
            {
                return tag_name.0.as_str().to_string();
            }
            name.as_str().to_string()
        }
        Ty::Opaque(name) => name.as_str().to_string(),
        Ty::Record {
            name,
            fields,
            resolved_params,
        } => {
            if let Some(surface) = generic_record_surface_local(
                name,
                fields,
                resolved_params.as_deref(),
                tag_types,
                tags,
            ) {
                return surface;
            }
            if tag_types.contains_key(&TagId(*name)) && !tag_has_generic_params_local(name, tags) {
                return name.as_str().to_string();
            }
            format_ty_for_hover(ty)
        }
        other => format_ty_for_hover(other),
    }
}

fn tag_has_generic_params(
    name: &Intern<String>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> bool {
    tag_params
        .and_then(|tp| tp.get(name))
        .is_some_and(|params| {
            params
                .iter()
                .any(|(_, k)| matches!(k.kind, ParameterKind::Generic))
        })
}

fn tag_has_generic_params_local(name: &Intern<String>, tags: &HashMap<TagId, TypedTag>) -> bool {
    tags.get(&TagId(*name))
        .and_then(|tag| tag.params.as_ref())
        .is_some_and(|params| {
            params
                .iter()
                .any(|(_, kind)| matches!(kind.kind, ParameterKind::Generic))
        })
}

fn generic_record_surface(
    name: &Intern<String>,
    fields: &[(Intern<String>, Box<Ty>)],
    resolved_params: Option<&[(Intern<String>, TyArg)]>,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> Option<String> {
    if !tag_has_generic_params(name, tag_params) {
        return None;
    }
    match name.as_str() {
        "List" => {
            let elem = list_element_ty_from_resolved_params(resolved_params)
                .or_else(|| list_element_ty_from_record_fields(fields))?;
            Some(format!(
                "List({})",
                type_annotation_surface_for_hover(&elem, tag_types, tag_params)
            ))
        }
        _ => None,
    }
}

fn generic_record_surface_local(
    name: &Intern<String>,
    fields: &[(Intern<String>, Box<Ty>)],
    resolved_params: Option<&[(Intern<String>, TyArg)]>,
    tag_types: &HashMap<TagId, Ty>,
    tags: &HashMap<TagId, TypedTag>,
) -> Option<String> {
    if !tag_has_generic_params_local(name, tags) {
        return None;
    }
    match name.as_str() {
        "List" => Some(format!(
            "List({})",
            type_annotation_surface_for_hover_local(
                &list_element_ty_from_resolved_params(resolved_params)
                    .or_else(|| list_element_ty_from_record_fields(fields))?,
                tag_types,
                tags,
            )
        )),
        _ => None,
    }
}

fn list_element_ty_from_resolved_params(params: Option<&[(Intern<String>, TyArg)]>) -> Option<Ty> {
    params?.iter().find_map(|(_, arg)| match arg {
        TyArg::Type(ty) => Some((**ty).clone()),
        TyArg::Const(_) => None,
    })
}

fn is_capitalized_type_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

fn list_element_ty_from_record_fields(fields: &[(Intern<String>, Box<Ty>)]) -> Option<Ty> {
    let (_, pointer) = fields.iter().find(|(n, _)| n.as_str() == "pointer")?;
    if let Some(pointee) = pointer.pointee_ty() {
        return Some(pointee.clone());
    }
    match pointer.as_ref() {
        Ty::Opaque(name) if is_capitalized_type_name(name.as_str()) => {
            Some(pointer.as_ref().clone())
        }
        Ty::Record { name, .. }
            if is_capitalized_type_name(name.as_str()) && name.as_str() != "List" =>
        {
            Some(pointer.as_ref().clone())
        }
        _ => None,
    }
}

/// Format a `Ty` for hover display.
pub fn format_ty_for_hover(ty: &Ty) -> String {
    match ty {
        Ty::Named { .. } => ty.format_for_hover(),
        Ty::AnonymousInteger { .. } => ty.format_for_hover(),
        Ty::ResultFamily { .. } => ty.format_for_hover(),
        Ty::Address { .. } => ty.format_for_hover(),
        Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer) => "integer literal".to_string(),
        Ty::Float { value } => {
            if let Some(HashFloat(v)) = value {
                format!("f64 = {}", v)
            } else {
                "f64".to_string()
            }
        }
        Ty::Unit => "()".to_string(),
        Ty::Record { fields, .. } => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(fname, fty)| format!("{}: {}", fname.as_str(), format_ty_for_hover(fty)))
                .collect();
            parts.join(", ")
        }
        Ty::Union { name, .. } => format!("Union({})", name.as_str()),
        Ty::Opaque(name) => name.as_str().to_string(),
        Ty::Array { elem, size } => format!("[{}; {}]", format_ty_for_hover(elem), size),
        Ty::Ptr { inner } => format!("*{}", format_ty_for_hover(inner)),
        Ty::Ref { inner, mutable } => {
            let prefix = if *mutable { "mut " } else { "ref " };
            format!("{}{}", prefix, format_ty_for_hover(inner))
        }
        Ty::Tuple(tys) => {
            let parts: Vec<String> = tys.iter().map(format_ty_for_hover).collect();
            format!("({})", parts.join(", "))
        }
        Ty::Literal(cv) => match cv {
            ast::ConstValue::String(s) => format!("'{s}'"),
            other => other.to_hover_string(),
        },
    }
}
