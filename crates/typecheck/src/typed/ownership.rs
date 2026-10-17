use super::*;
use crate::solver::{Predicate, ProveResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlap {
    Disjoint,
    Equal,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct GroupId(pub u32);

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
    Static,
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
    pub(crate) actuals: HashMap<GroupId, ReferenceTargetSet>,
    pub(crate) grouped_argument_counts: HashMap<GroupId, usize>,
    pub(crate) grouped_argument_mutate: HashMap<GroupId, bool>,
    pub(crate) grouped_argument_observe: HashMap<GroupId, bool>,
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
        let mut current = target;
        let mut projections = Vec::new();
        while let Some(base) = current.parent() {
            projections.push(current);
            current = base;
        }
        let mut effect = match current {
            ReferenceTargetGroup::Param(group) => Self::Group(*group),
            ReferenceTargetGroup::Local(_) | ReferenceTargetGroup::Static => return None,
            _ => unreachable!("a reference target without a parent is a root"),
        };
        for projection in projections.into_iter().rev() {
            effect = match projection {
                ReferenceTargetGroup::Field { index, .. } => Self::Field {
                    base: Box::new(effect),
                    index: *index,
                },
                ReferenceTargetGroup::Variant { name, .. } => Self::Variant {
                    base: Box::new(effect),
                    name: *name,
                },
                ReferenceTargetGroup::ItemRegion { index, .. } => Self::ItemRegion {
                    base: Box::new(effect),
                    index: index.clone(),
                },
                ReferenceTargetGroup::ItemRange { start, end, .. } => Self::ItemRange {
                    base: Box::new(effect),
                    start: start.clone(),
                    end: end.clone(),
                },
                ReferenceTargetGroup::Deref(_) => Self::Deref(Box::new(effect)),
                ReferenceTargetGroup::Param(_)
                | ReferenceTargetGroup::Local(_)
                | ReferenceTargetGroup::Static => {
                    unreachable!("a reference projection has a parent")
                }
            };
        }
        Some(effect)
    }

    pub fn substitute(
        &self,
        group: GroupId,
        actual: &ReferenceTargetGroup,
    ) -> Option<AppliedEffectTarget> {
        let mut current = self;
        let mut projections = Vec::new();
        let mut target = loop {
            match current {
                Self::Group(effect_group) => {
                    if *effect_group != group {
                        return None;
                    }
                    break actual.clone();
                }
                Self::Field { base, .. }
                | Self::Variant { base, .. }
                | Self::ItemRegion { base, .. }
                | Self::ItemRange { base, .. }
                | Self::Deref(base)
                | Self::Descendants(base) => {
                    projections.push(current);
                    current = base;
                }
            }
        };
        let mut descendants = false;
        for projection in projections.into_iter().rev() {
            target = match projection {
                Self::Field { index, .. } => ReferenceTargetGroup::Field {
                    base: Box::new(target),
                    index: *index,
                },
                Self::Variant { name, .. } => ReferenceTargetGroup::Variant {
                    base: Box::new(target),
                    name: *name,
                },
                Self::ItemRegion { index, .. } => ReferenceTargetGroup::ItemRegion {
                    base: Box::new(target),
                    index: index.clone(),
                },
                Self::ItemRange { start, end, .. } => ReferenceTargetGroup::ItemRange {
                    base: Box::new(target),
                    start: start.clone(),
                    end: end.clone(),
                },
                Self::Deref(_) => ReferenceTargetGroup::Deref(Box::new(target)),
                Self::Descendants(_) => {
                    descendants = true;
                    target
                }
                Self::Group(_) => unreachable!("an effect projection is not a root group"),
            };
        }
        Some(AppliedEffectTarget {
            target,
            descendants,
        })
    }
}

impl ReferenceTargetGroup {
    fn root(&self) -> &Self {
        let mut current = self;
        while let Some(parent) = current.parent() {
            current = parent;
        }
        current
    }

    pub fn is_static(&self) -> bool {
        matches!(self.root(), Self::Static)
    }

    pub fn root_local(&self) -> Option<PlaceId> {
        match self.root() {
            Self::Local(place) => Some(*place),
            _ => None,
        }
    }

    pub fn root_param(&self) -> Option<GroupId> {
        match self.root() {
            Self::Param(group) => Some(*group),
            _ => None,
        }
    }

    pub fn matches_projection_from(
        &self,
        ancestor: &Self,
        projections: &[GroupProjection],
        constraints: &ConstraintEnv,
    ) -> bool {
        let mut current = self;
        for projection in projections.iter().rev() {
            current = match (current, projection) {
                (
                    Self::Field {
                        base,
                        index: actual,
                        ..
                    },
                    GroupProjection::Field { index, .. },
                ) if actual == index => base,
                (Self::Variant { base, name: actual }, GroupProjection::Variant { name })
                    if actual == name =>
                {
                    base
                }
                (
                    Self::ItemRegion { base, .. } | Self::ItemRange { base, .. },
                    GroupProjection::Items,
                )
                | (Self::Deref(base), GroupProjection::Pointee) => base,
                _ => return false,
            };
        }
        current.overlap(ancestor, constraints) == Overlap::Equal
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

    pub(crate) fn is_within_items_of(&self, ancestor: &Self, constraints: &ConstraintEnv) -> bool {
        let mut target = Some(self);
        while let Some(current) = target {
            if matches!(
                current,
                Self::ItemRegion { base, .. } | Self::ItemRange { base, .. }
                    if base.overlap(ancestor, constraints) == Overlap::Equal
            ) {
                return true;
            }
            target = current.parent();
        }
        false
    }

    pub fn overlap(&self, other: &Self, constraints: &ConstraintEnv) -> Overlap {
        let left_depth = self.depth();
        let right_depth = other.depth();
        let mut left = self;
        let mut right = other;
        for _ in 0..left_depth.saturating_sub(right_depth) {
            left = left.parent().expect("positive target depth has a parent");
        }
        for _ in 0..right_depth.saturating_sub(left_depth) {
            right = right.parent().expect("positive target depth has a parent");
        }
        let mut pairs = Vec::new();
        while let (Some(left_parent), Some(right_parent)) = (left.parent(), right.parent()) {
            pairs.push((left, right));
            left = left_parent;
            right = right_parent;
        }
        let mut overlap = match (left, right) {
            (Self::Static, Self::Static) => Overlap::Equal,
            (Self::Local(left), Self::Local(right)) if left == right => Overlap::Equal,
            (Self::Param(left), Self::Param(right)) if left == right => Overlap::Equal,
            _ => Overlap::Disjoint,
        };
        for (left, right) in pairs.into_iter().rev() {
            if overlap != Overlap::Equal {
                return overlap;
            }
            overlap = match (left, right) {
                (Self::Field { index: left, .. }, Self::Field { index: right, .. }) => {
                    if left == right {
                        Overlap::Equal
                    } else {
                        Overlap::Disjoint
                    }
                }
                (Self::Variant { name: left, .. }, Self::Variant { name: right, .. }) => {
                    if left == right {
                        Overlap::Equal
                    } else {
                        Overlap::Disjoint
                    }
                }
                (Self::ItemRegion { index: left, .. }, Self::ItemRegion { index: right, .. }) => {
                    index_overlap(left, right, constraints)
                }
                (Self::ItemRegion { index, .. }, Self::ItemRange { start, end, .. })
                | (Self::ItemRange { start, end, .. }, Self::ItemRegion { index, .. }) => {
                    index_range_overlap(index, start, end, constraints)
                }
                (
                    Self::ItemRange {
                        start: left_start,
                        end: left_end,
                        ..
                    },
                    Self::ItemRange {
                        start: right_start,
                        end: right_end,
                        ..
                    },
                ) => range_overlap(left_start, left_end, right_start, right_end, constraints),
                (Self::Deref(_), Self::Deref(_)) => Overlap::Equal,
                _ => Overlap::Partial,
            };
        }
        if left_depth != right_depth {
            descendant_overlap(overlap)
        } else {
            overlap
        }
    }

    fn depth(&self) -> usize {
        let mut current = self;
        let mut depth = 0;
        while let Some(parent) = current.parent() {
            depth += 1;
            current = parent;
        }
        depth
    }

    fn parent(&self) -> Option<&Self> {
        match self {
            Self::Field { base, .. }
            | Self::Variant { base, .. }
            | Self::ItemRegion { base, .. }
            | Self::ItemRange { base, .. }
            | Self::Deref(base) => Some(base),
            Self::Param(_) | Self::Local(_) | Self::Static => None,
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
