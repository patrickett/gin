use std::collections::{HashMap, HashSet};

use crate::span::{HasSpanId, SpanId};

use internment::Intern;

pub use crate::analysis::const_value::{Bound, ConstValue, TypeConstraint};

/// The state of a variable during flow analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VarState {
    /// Variable is alive and usable.
    Alive,
    /// Variable has been moved (ownership consumed).
    Moved,
    /// Variable has been moved but the slot is alive (for `:` bindings, the slot can be reassigned).
    MovedButSlotAlive,
}

/// Capability level for a variable's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Capability {
    /// Read capability — always held implicitly.
    Read,
    /// Write capability — acquired via `mut`.
    Write,
    /// Own capability — acquired via `own` or construction.
    Own,
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
    /// Track whether each variable is alive or moved.
    var_states: HashMap<Intern<String>, VarState>,
    /// Capabilities held for each variable.
    capabilities: HashMap<Intern<String>, Capability>,
    /// Maps each variable to its region identifier (for invalidation tracking).
    region_owner: HashMap<Intern<String>, Intern<String>>,
    /// Regions that have been consumed (variables moved out of).
    consumed_regions: HashSet<Intern<String>>,
}

impl PartialEq for FlowContext {
    fn eq(&self, other: &Self) -> bool {
        self.constraints == other.constraints
            && self.constants == other.constants
            && self.var_states == other.var_states
            && self.capabilities == other.capabilities
    }
}

impl FlowContext {
    pub fn new() -> Self {
        Self {
            constraints: HashMap::new(),
            constants: HashMap::new(),
            parent: None,
            var_states: HashMap::new(),
            capabilities: HashMap::new(),
            region_owner: HashMap::new(),
            consumed_regions: HashSet::new(),
        }
    }

    pub fn with_parent(parent: FlowContext) -> Self {
        Self {
            constraints: HashMap::new(),
            constants: HashMap::new(),
            parent: Some(Box::new(parent)),
            var_states: HashMap::new(),
            capabilities: HashMap::new(),
            region_owner: HashMap::new(),
            consumed_regions: HashSet::new(),
        }
    }

    /// Narrow a variable based on a pattern match.
    pub fn narrow(&mut self, var: Intern<String>, constraint: TypeConstraint) {
        self.constraints.insert(var, constraint);
    }

    /// Check if a narrowing is impossible given current constraints.
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

    /// Get the constraint for a variable, if any.
    pub fn get_constraint(&self, var: &Intern<String>) -> Option<&TypeConstraint> {
        self.constraints
            .get(var)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_constraint(var)))
    }

    /// Reset a variable's narrowing (on reassignment).
    pub fn reset(&mut self, var: &Intern<String>) {
        self.constraints.remove(var);
        self.constants.remove(var);
        self.reset_ownership(var);
    }

    /// Check if this context has a local (non-inherited) constraint for a variable.
    pub fn has_local_constraint(&self, var: &Intern<String>) -> bool {
        self.constraints.contains_key(var)
    }

    /// Get all variables with local constraints.
    pub fn local_constraints(&self) -> impl Iterator<Item = (&Intern<String>, &TypeConstraint)> {
        self.constraints.iter()
    }

    /// Record a known constant value for a variable.
    pub fn set_constant(&mut self, var: Intern<String>, value: ConstValue) {
        self.constants.insert(var, value);
    }

    /// Get the known constant value for a variable, if any.
    pub fn get_constant(&self, var: &Intern<String>) -> Option<&ConstValue> {
        self.constants
            .get(var)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_constant(var)))
    }

    /// Reset a variable's constant value (on reassignment).
    pub fn reset_constant(&mut self, var: &Intern<String>) {
        self.constants.remove(var);
    }

    /// Get all variables with local constant values.
    pub fn local_constants(&self) -> impl Iterator<Item = (&Intern<String>, &ConstValue)> {
        self.constants.iter()
    }

    // --- Ownership tracking accessors ---

    /// Set the state of a variable (Alive, Moved, etc.).
    pub fn set_var_state(&mut self, var: Intern<String>, state: VarState) {
        self.var_states.insert(var, state);
    }

    /// Get the state of a variable.
    pub fn get_var_state(&self, var: &Intern<String>) -> Option<VarState> {
        self.var_states
            .get(var)
            .copied()
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_var_state(var)))
    }

    /// Set the capability for a variable.
    pub fn set_capability(&mut self, var: Intern<String>, cap: Capability) {
        self.capabilities.insert(var, cap);
    }

    /// Get the capability for a variable.
    pub fn get_capability(&self, var: &Intern<String>) -> Option<Capability> {
        self.capabilities
            .get(var)
            .copied()
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_capability(var)))
    }

    /// Set the region owner for a variable.
    pub fn set_region_owner(&mut self, var: Intern<String>, region: Intern<String>) {
        self.region_owner.insert(var, region);
    }

    /// Get the region owner for a variable.
    pub fn get_region_owner(&self, var: &Intern<String>) -> Option<&Intern<String>> {
        self.region_owner
            .get(var)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_region_owner(var)))
    }

    /// Mark a region as consumed.
    pub fn consume_region(&mut self, region: &Intern<String>) {
        self.consumed_regions.insert(*region);
    }

    /// Check if a region has been consumed.
    pub fn is_region_consumed(&self, region: &Intern<String>) -> bool {
        self.consumed_regions.contains(region)
            || self
                .parent
                .as_ref()
                .is_some_and(|p| p.is_region_consumed(region))
    }

    /// Reset ownership tracking for a variable (on reassignment).
    pub fn reset_ownership(&mut self, var: &Intern<String>) {
        self.var_states.remove(var);
        self.capabilities.remove(var);
        self.region_owner.remove(var);
    }
}

impl Default for FlowContext {
    fn default() -> Self {
        Self::new()
    }
}

// NOTE: Constant propagation is implemented in FlowAnalyzer via eval_const /
// extract_pattern_constants / extract_comparisons.
// - `val Maybe(3): Some(3)` → val is Some(3)
// - `if val is Some(v)` / `when val is Some(v) then …` pattern extraction → v = 3
// - `four: v + 1` → four = 4 via constant folding
// - `if num < 10` / `while i < len` → comparison narrowing inside body, negated after
// TODO: constant propagation through reassignment in loops (i: i + 1 where i = 0)
// TODO: Cross-function type narrowing — functions narrowing return types based on conditions.
// e.g. `less_than_ten(num Int) Maybe[Int]` should let callers know the Int payload is `< 10`.
// This requires encoding the function's postconditions (derived from its flow analysis) into
// a "contract" that the caller's flow analyzer can apply when it sees the call result.
// A lighter first step: store the narrowed constraints from each `return` in FlowAnalysis
// so hover on the function itself can show e.g. `less_than_ten(num Int) Maybe(Int < 10)`.
//
// TODO: Return type inference from early-returning blocks.
// e.g. `main` with `return four` (where four = 4) and a final `val` (where val = None)
// should infer the return type as `Int or None` and show `main 4 or Nothing` on hover.
// This requires collecting the return-site types from all `return` expressions and the
// implicit trailing expression, then computing their union.

/// Result of flow analysis on a function body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlowAnalysis {
    /// Map from AST node index to its flow context.
    pub expr_contexts: HashMap<usize, FlowContext>,
    /// Map from SpanId to expression index, for position-aware context lookup.
    pub expr_spans: HashMap<SpanId, usize>,
    /// Impossible checks detected (for diagnostics).
    pub impossible_checks: Vec<ImpossibleCheck>,
    /// Index out of bounds accesses detected (for diagnostics).
    pub bounds_checks: Vec<IndexOutOfBounds>,
    /// The flow context at the end of each function body (after all narrowing).
    pub final_context: FlowContext,
    /// All variants of each union type: union_name → [variant_names].
    pub union_to_variants: HashMap<Intern<String>, Vec<Intern<String>>>,
}

impl FlowAnalysis {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the flow context at a given expression index.
    pub fn get_context(&self, index: usize) -> Option<&FlowContext> {
        self.expr_contexts.get(&index)
    }

    /// Add a flow context for an expression at the given index.
    pub fn insert_context(&mut self, index: usize, ctx: FlowContext) {
        self.expr_contexts.insert(index, ctx);
    }

    /// Record the SpanId for an expression index.
    pub fn insert_span(&mut self, span_id: SpanId, index: usize) {
        self.expr_spans.insert(span_id, index);
    }

    /// Get the narrowed constraint for a variable at a given expression index.
    ///
    /// Returns `None` if there is no narrowing for that variable at that point.
    pub fn narrowed_at(&self, expr_index: usize, var_name: &str) -> Option<&TypeConstraint> {
        let ctx = self.expr_contexts.get(&expr_index)?;
        let var = Intern::<String>::from_ref(var_name);
        ctx.get_constraint(&var)
    }

    /// Get the known constant value for a variable at a given expression index.
    pub fn value_at(&self, expr_index: usize, var_name: &str) -> Option<&ConstValue> {
        let ctx = self.expr_contexts.get(&expr_index)?;
        let var = Intern::<String>::from_ref(var_name);
        ctx.get_constant(&var)
    }

    /// Add an impossible check to the diagnostics list.
    pub fn add_impossible_check(&mut self, check: ImpossibleCheck) {
        self.impossible_checks.push(check);
    }

    /// Add an index out of bounds check to the diagnostics list.
    pub fn add_bounds_check(&mut self, check: IndexOutOfBounds) {
        self.bounds_checks.push(check);
    }

    /// Get the display string for the narrowed type of `var_name` based on the final context.
    ///
    /// Returns `None` if no narrowing is in effect.
    pub fn narrowed_type_string(&self, var_name: &str) -> Option<String> {
        let var = Intern::<String>::from_ref(var_name);
        self.constraint_to_display(self.final_context.get_constraint(&var)?)
    }

    /// Returns `(union_name, variant_name)` for the positive narrowing inside the if block.
    ///
    /// i.e., for `if val is Some(v)` with an early return, returns `(Maybe, Some)`.
    pub fn inside_if_variant(&self, var_name: &str) -> Option<(Intern<String>, Intern<String>)> {
        let var = Intern::<String>::from_ref(var_name);
        match self.final_context.get_constraint(&var)? {
            // final_context has IsNotVariant → inside-if the variable IS that variant
            TypeConstraint::IsNotVariant(union, variant) => Some((*union, *variant)),
            _ => None,
        }
    }

    fn constraint_to_display(&self, constraint: &TypeConstraint) -> Option<String> {
        match constraint {
            TypeConstraint::IsVariant(union, variant) => {
                Some(format!("{}.{}", union.as_str(), variant.as_str()))
            }
            TypeConstraint::IsNotVariant(union, excluded) => {
                let variants = self.union_to_variants.get(union)?;
                let mut remaining: Vec<String> = variants
                    .iter()
                    .filter(|v| *v != excluded)
                    .map(|v| format!("{}.{}", union.as_str(), v.as_str()))
                    .collect();
                if remaining.is_empty() {
                    return None;
                }
                remaining.sort();
                Some(remaining.join(" or "))
            }
            TypeConstraint::Compare { op, bound } => {
                let bound_str = match bound {
                    Bound::Variable(name) => name.as_str().to_string(),
                    Bound::Constant(val) => val.to_hover_string(),
                };
                Some(format!("{} {}", op.symbol(), bound_str))
            }
        }
    }
}

/// Represents an impossible type check detected during analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpossibleCheck {
    /// The expression index where the impossible check occurs.
    pub expr_index: usize,
    /// Human-readable reason why this check is impossible.
    pub reason: String,
}

/// Represents an index out of bounds access detected during analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexOutOfBounds {
    /// The expression index where the out-of-bounds access occurs.
    pub expr_index: usize,
    /// The source span of the buffer access expression.
    pub span: SpanId,
    /// The index value being accessed.
    pub index: i128,
    /// The size of the buffer.
    pub size: usize,
}

impl HasSpanId for IndexOutOfBounds {
    fn span_id(&self) -> SpanId {
        self.span
    }
}
