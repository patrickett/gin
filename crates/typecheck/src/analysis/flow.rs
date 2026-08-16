use std::collections::{HashMap, HashSet};

use crate::solver::ConstraintEnv;
use crate::typed::PlaceId;

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
    /// Track whether each variable is alive or moved.
    var_states: HashMap<PlaceId, VarState>,
}

impl PartialEq for FlowContext {
    fn eq(&self, other: &Self) -> bool {
        self.var_states == other.var_states && self.constraint_env == other.constraint_env
    }
}

impl FlowContext {
    pub fn new() -> Self {
        Self {
            var_states: HashMap::new(),
            constraint_env: ConstraintEnv::default(),
        }
    }

    pub fn set_place_state(&mut self, place: PlaceId, state: VarState) {
        self.var_states.insert(place, state);
    }

    pub fn place_state(&self, place: PlaceId) -> Option<VarState> {
        self.var_states.get(&place).copied()
    }

    pub fn merge_branch_states(&mut self, branches: &[Self]) {
        let places: HashSet<_> = branches
            .iter()
            .flat_map(|branch| branch.var_states.keys().copied())
            .chain(self.var_states.keys().copied())
            .collect();
        for place in places {
            let states: Vec<_> = branches
                .iter()
                .filter_map(|branch| branch.place_state(place))
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
