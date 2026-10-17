use std::collections::{HashMap, HashSet};

use crate::solver::ConstraintEnv;
use crate::typed::{ExprId, MathematicalComparison, PlaceId, ReferenceTargetSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ComparisonOutcome {
    Proposition(bool),
    Alternative(internment::Intern<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScalarComparison {
    pub outcome: ComparisonOutcome,
    pub left: ast::NormalExpr,
    pub right: ast::NormalExpr,
    pub comparison: MathematicalComparison,
}

/// The state of a variable during flow analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VarState {
    /// Variable is alive and usable.
    Alive,
    /// Variable has been consumed (ownership transferred).
    Consumed,
    /// Variable is consumed on at least one incoming control-flow path.
    MaybeConsumed,
    /// Variable has been declared with a type but not yet assigned a value.
    /// Reading a variable in this state is a flaw.
    Declared,
    Invalidated,
    MaybeInvalidated,
}

/// Flow-sensitive type information at a program point.
#[derive(Debug, Clone)]
pub struct FlowContext {
    /// Known constant constraints for refinement predicate solving.
    pub constraint_env: ConstraintEnv,
    pub(crate) record_targets: HashMap<internment::Intern<String>, ReferenceTargetSet>,
    pub(crate) raw_exposed: ReferenceTargetSet,
    pub(crate) scalar_bindings: HashSet<internment::Intern<String>>,
    pub(crate) binding_types: HashMap<internment::Intern<String>, crate::ty::Ty>,
    pub(crate) scalar_values: HashMap<ExprId, ast::NormalExpr>,
    pub(crate) comparisons: HashMap<ExprId, Vec<ScalarComparison>>,
    pub(crate) binding_comparisons: HashMap<internment::Intern<String>, Vec<ScalarComparison>>,
    /// Track whether each variable is alive or moved.
    var_states: HashMap<PlaceId, VarState>,
    pub(crate) excluded_union_variants: HashMap<PlaceId, HashSet<internment::Intern<String>>>,
}

impl PartialEq for FlowContext {
    fn eq(&self, other: &Self) -> bool {
        self.var_states == other.var_states
            && self.excluded_union_variants == other.excluded_union_variants
            && self.constraint_env == other.constraint_env
            && self.record_targets == other.record_targets
            && self.raw_exposed == other.raw_exposed
            && self.scalar_bindings == other.scalar_bindings
            && self.binding_types == other.binding_types
            && self.scalar_values == other.scalar_values
            && self.comparisons == other.comparisons
            && self.binding_comparisons == other.binding_comparisons
    }
}

impl FlowContext {
    pub fn new() -> Self {
        Self {
            var_states: HashMap::new(),
            excluded_union_variants: HashMap::new(),
            constraint_env: ConstraintEnv::default(),
            record_targets: HashMap::new(),
            raw_exposed: ReferenceTargetSet::default(),
            scalar_bindings: HashSet::new(),
            binding_types: HashMap::new(),
            scalar_values: HashMap::new(),
            comparisons: HashMap::new(),
            binding_comparisons: HashMap::new(),
        }
    }

    pub fn set_place_state(&mut self, place: PlaceId, state: VarState) {
        if !matches!(state, VarState::Consumed | VarState::MaybeConsumed) {
            self.excluded_union_variants.remove(&place);
        }
        self.var_states.insert(place, state);
    }

    pub fn place_state(&self, place: PlaceId) -> Option<VarState> {
        self.var_states.get(&place).copied()
    }

    fn merge_union_variant_exclusions(&mut self, branches: &[Self]) {
        if let Some(first) = branches.first() {
            let mut excluded = first.excluded_union_variants.clone();
            for branch in branches.iter().skip(1) {
                for (place, variants) in &branch.excluded_union_variants {
                    excluded.entry(*place).or_default().extend(variants);
                }
            }
            excluded.retain(|place, variants| {
                variants.retain(|variant| {
                    branches.iter().all(|branch| {
                        branch.place_state(*place) == Some(VarState::Consumed)
                            || branch
                                .excluded_union_variants
                                .get(place)
                                .is_some_and(|excluded| excluded.contains(variant))
                    })
                });
                !variants.is_empty()
            });
            self.excluded_union_variants = excluded;
        }
    }

    pub fn merge_branch_states(&mut self, branches: &[Self]) {
        self.merge_union_variant_exclusions(branches);
        for branch in branches {
            self.raw_exposed.extend(&branch.raw_exposed);
        }
        self.scalar_values.retain(|id, value| {
            branches
                .iter()
                .all(|branch| branch.scalar_values.get(id) == Some(value))
        });
        self.comparisons.retain(|id, value| {
            branches
                .iter()
                .all(|branch| branch.comparisons.get(id) == Some(value))
        });
        self.binding_comparisons.retain(|name, value| {
            branches
                .iter()
                .all(|branch| branch.binding_comparisons.get(name) == Some(value))
        });
        self.binding_types.retain(|name, ty| {
            branches
                .iter()
                .all(|branch| branch.binding_types.get(name) == Some(ty))
        });
        self.scalar_bindings.retain(|name| {
            branches
                .iter()
                .all(|branch| branch.scalar_bindings.contains(name))
        });
        let names: HashSet<_> = branches
            .iter()
            .flat_map(|branch| branch.record_targets.keys().copied())
            .chain(self.record_targets.keys().copied())
            .collect();
        for name in names {
            let mut joined = ReferenceTargetSet::default();
            let mut complete = true;
            for branch in branches {
                if let Some(targets) = branch
                    .record_targets
                    .get(&name)
                    .or_else(|| self.record_targets.get(&name))
                {
                    joined.extend(targets);
                } else {
                    complete = false;
                }
            }
            if complete && joined.iter().next().is_some() {
                self.record_targets.insert(name, joined);
            } else {
                self.record_targets.remove(&name);
            }
        }
        let places: HashSet<_> = branches
            .iter()
            .flat_map(|branch| branch.var_states.keys().copied())
            .chain(self.var_states.keys().copied())
            .collect();
        for place in places {
            let states: Vec<_> = branches
                .iter()
                .map(|branch| {
                    branch
                        .place_state(place)
                        .unwrap_or_else(|| self.place_state(place).unwrap_or(VarState::Alive))
                })
                .collect();
            let Some(first) = states.first().copied() else {
                continue;
            };
            let state = if states.iter().all(|state| *state == first) {
                first
            } else if states
                .iter()
                .any(|state| matches!(state, VarState::Consumed | VarState::MaybeConsumed))
            {
                VarState::MaybeConsumed
            } else if states
                .iter()
                .any(|state| matches!(state, VarState::Invalidated | VarState::MaybeInvalidated))
            {
                VarState::MaybeInvalidated
            } else if states.contains(&VarState::Declared) {
                VarState::Declared
            } else {
                VarState::Alive
            };
            self.var_states.insert(place, state);
        }
    }
}

impl Default for FlowContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_spelled_bindings_keep_independent_place_state() {
        let outer = PlaceId(3);
        let inner = PlaceId(8);
        let mut flow = FlowContext::new();

        flow.set_place_state(outer, VarState::Consumed);
        flow.set_place_state(inner, VarState::Alive);

        assert_eq!(flow.place_state(outer), Some(VarState::Consumed));
        assert_eq!(flow.place_state(inner), Some(VarState::Alive));
    }
}
