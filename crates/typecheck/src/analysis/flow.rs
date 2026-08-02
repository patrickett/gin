use std::collections::{HashMap, HashSet};

use internment::Intern;

use crate::solver::ConstraintEnv;
use ast::{ConstValue, TypeConstraint};

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
    /// Map from variable name to its narrowed type constraints.
    constraints: HashMap<Intern<String>, TypeConstraint>,
    /// Map from variable name to its known constant value.
    constants: HashMap<Intern<String>, ConstValue>,
    /// Parent context for nested scopes (blocks, loops, etc.).
    parent: Option<Box<FlowContext>>,
    /// Known constant constraints for refinement predicate solving.
    pub constraint_env: ConstraintEnv,
    /// Track whether each variable is alive or moved.
    var_states: HashMap<Intern<String>, VarState>,
}

impl PartialEq for FlowContext {
    fn eq(&self, other: &Self) -> bool {
        self.constraints == other.constraints
            && self.constants == other.constants
            && self.var_states == other.var_states
            && self.constraint_env == other.constraint_env
    }
}

fn mentions_var(expr: &ast::NormalExpr, var: &Intern<String>) -> bool {
    match expr {
        ast::NormalExpr::Var(name) => name == var,
        ast::NormalExpr::Add(l, r) | ast::NormalExpr::Sub(l, r) | ast::NormalExpr::Mul(l, r) => {
            mentions_var(l, var) || mentions_var(r, var)
        }
        _ => false,
    }
}

impl FlowContext {
    pub fn new() -> Self {
        Self {
            constraints: HashMap::new(),
            constants: HashMap::new(),
            parent: None,
            var_states: HashMap::new(),
            constraint_env: ConstraintEnv::default(),
        }
    }

    pub fn with_parent(parent: FlowContext) -> Self {
        Self {
            constraints: HashMap::new(),
            constants: HashMap::new(),
            parent: Some(Box::new(parent)),
            var_states: HashMap::new(),
            constraint_env: ConstraintEnv::default(),
        }
    }

    pub fn narrow(&mut self, var: Intern<String>, constraint: TypeConstraint) {
        self.constraints.insert(var, constraint);
    }

    pub fn is_impossible(&self, var: &Intern<String>, check_constraint: &TypeConstraint) -> bool {
        if let Some(existing) = self.constraints.get(var)
            && existing.contradicts(check_constraint)
        {
            return true;
        }
        if let Some(parent) = &self.parent {
            parent.is_impossible(var, check_constraint)
        } else {
            false
        }
    }

    pub fn get_constraint(&self, var: &Intern<String>) -> Option<&TypeConstraint> {
        self.constraints
            .get(var)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_constraint(var)))
    }

    /// Reset a variable's narrowing (on reassignment).
    pub fn reset(&mut self, var: &Intern<String>) {
        self.constraints.remove(var);
        self.constants.remove(var);
    }

    /// Check if this context has a local (non-inherited) constraint for a variable.
    pub fn has_local_constraint(&self, var: &Intern<String>) -> bool {
        self.constraints.contains_key(var)
    }

    pub fn local_constraints(&self) -> impl Iterator<Item = (&Intern<String>, &TypeConstraint)> {
        self.constraints.iter()
    }

    pub fn set_constant(&mut self, var: Intern<String>, value: ConstValue) {
        self.constants.insert(var, value);
    }

    pub fn get_constant(&self, var: &Intern<String>) -> Option<&ConstValue> {
        self.constants
            .get(var)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_constant(var)))
    }

    /// Reset a variable's constant value (on reassignment).
    pub fn reset_constant(&mut self, var: &Intern<String>) {
        self.constants.remove(var);
    }

    /// Record a known less-than fact: `lhs < rhs`.
    pub fn add_known_lt(&mut self, lhs: ast::NormalExpr, rhs: ast::NormalExpr) {
        self.constraint_env.known_lt.push((lhs, rhs));
    }

    /// Record a known equal fact: `lhs == rhs`.
    pub fn add_known_eq(&mut self, lhs: ast::NormalExpr, rhs: ast::NormalExpr) {
        self.constraint_env.known_eq.push((lhs, rhs));
    }

    /// Remove all facts involving a variable (on reassignment).
    pub fn remove_facts_for(&mut self, var: &Intern<String>) {
        self.constraint_env
            .known_lt
            .retain(|(l, r)| !mentions_var(l, var) && !mentions_var(r, var));
        self.constraint_env
            .known_eq
            .retain(|(l, r)| !mentions_var(l, var) && !mentions_var(r, var));
    }

    pub fn local_constants(&self) -> impl Iterator<Item = (&Intern<String>, &ConstValue)> {
        self.constants.iter()
    }

    pub fn local_var_states(&self) -> impl Iterator<Item = (&Intern<String>, &VarState)> {
        self.var_states.iter()
    }

    pub fn set_var_state(&mut self, var: Intern<String>, state: VarState) {
        self.var_states.insert(var, state);
    }

    pub fn get_var_state(&self, var: &Intern<String>) -> Option<VarState> {
        self.var_states
            .get(var)
            .copied()
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_var_state(var)))
    }

    pub fn merge_branch_states(&mut self, branches: &[Self]) {
        let names: HashSet<_> = branches
            .iter()
            .flat_map(|branch| branch.var_states.keys().copied())
            .chain(self.var_states.keys().copied())
            .collect();
        for name in names {
            let states: Vec<_> = branches
                .iter()
                .filter_map(|branch| branch.get_var_state(&name))
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
            self.var_states.insert(name, state);
        }
    }
}

impl Default for FlowContext {
    fn default() -> Self {
        Self::new()
    }
}
