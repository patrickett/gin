use super::*;

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
    pub fn root_local(&self) -> Option<PlaceId> {
        match self {
            Self::Local(place) => Some(*place),
            Self::Param(_) => None,
            Self::Field { base, .. }
            | Self::Variant { base, .. }
            | Self::ItemRegion { base, .. }
            | Self::ItemRange { base, .. }
            | Self::Deref(base) => base.root_local(),
        }
    }

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
