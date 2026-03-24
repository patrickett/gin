use std::collections::HashMap;

use crate::intern::IStr;

/// Represents a narrowed type constraint on a variable.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeConstraint {
    /// Variable is definitely a specific variant of a union.
    IsVariant(IStr, IStr),
    /// Variable is definitely NOT a specific variant.
    IsNotVariant(IStr, IStr),
}

impl TypeConstraint {
    /// Returns true if this constraint contradicts another.
    pub fn contradicts(&self, other: &TypeConstraint) -> bool {
        match (self, other) {
            (TypeConstraint::IsVariant(u1, v1), TypeConstraint::IsVariant(u2, v2)) => {
                u1 == u2 && v1 != v2
            }
            (TypeConstraint::IsVariant(_, v1), TypeConstraint::IsNotVariant(_, v2)) => v1 == v2,
            (TypeConstraint::IsNotVariant(_, v1), TypeConstraint::IsVariant(_, v2)) => v1 == v2,
            _ => false,
        }
    }
}

/// Flow-sensitive type information at a program point.
#[derive(Debug, Clone)]
pub struct FlowContext {
    /// Map from variable name to its narrowed type constraints.
    constraints: HashMap<IStr, TypeConstraint>,
    /// Parent context for nested scopes (blocks, loops, etc.).
    parent: Option<Box<FlowContext>>,
}

impl PartialEq for FlowContext {
    fn eq(&self, other: &Self) -> bool {
        self.constraints == other.constraints
    }
}

impl FlowContext {
    pub fn new() -> Self {
        Self {
            constraints: HashMap::new(),
            parent: None,
        }
    }

    pub fn with_parent(parent: FlowContext) -> Self {
        Self {
            constraints: HashMap::new(),
            parent: Some(Box::new(parent)),
        }
    }

    /// Narrow a variable based on a pattern match.
    pub fn narrow(&mut self, var: IStr, constraint: TypeConstraint) {
        self.constraints.insert(var, constraint);
    }

    /// Check if a narrowing is impossible given current constraints.
    pub fn is_impossible(&self, var: &IStr, check_constraint: &TypeConstraint) -> bool {
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
    pub fn get_constraint(&self, var: &IStr) -> Option<&TypeConstraint> {
        self.constraints
            .get(var)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_constraint(var)))
    }

    /// Reset a variable's narrowing (on reassignment).
    pub fn reset(&mut self, var: &IStr) {
        self.constraints.remove(var);
    }

    /// Check if this context has a local (non-inherited) constraint for a variable.
    pub fn has_local_constraint(&self, var: &IStr) -> bool {
        self.constraints.contains_key(var)
    }

    /// Get all variables with local constraints.
    pub fn local_constraints(&self) -> impl Iterator<Item = (&IStr, &TypeConstraint)> {
        self.constraints.iter()
    }
}

impl Default for FlowContext {
    fn default() -> Self {
        Self::new()
    }
}

impl Eq for FlowContext {}

/// Result of flow analysis on a function body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowAnalysis {
    /// Map from AST node index to its flow context.
    pub expr_contexts: HashMap<usize, FlowContext>,
    /// Impossible checks detected (for diagnostics).
    pub impossible_checks: Vec<ImpossibleCheck>,
    /// The flow context at the end of each function body (after all narrowing).
    pub final_context: FlowContext,
    /// All variants of each union type: union_name → [variant_names].
    pub union_to_variants: HashMap<IStr, Vec<IStr>>,
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

    /// Add an impossible check to the diagnostics list.
    pub fn add_impossible_check(&mut self, check: ImpossibleCheck) {
        self.impossible_checks.push(check);
    }

    /// Get the display string for the narrowed type of `var_name` based on the final context.
    ///
    /// Returns `None` if no narrowing is in effect.
    pub fn narrowed_type_string(&self, var_name: &str) -> Option<String> {
        let var = IStr::new(var_name.to_string());
        self.constraint_to_display(self.final_context.get_constraint(&var)?)
    }

    /// Returns `(union_name, variant_name)` for the positive narrowing inside the if block.
    ///
    /// i.e., for `if val is Some(v)` with an early return, returns `(Maybe, Some)`.
    pub fn inside_if_variant(&self, var_name: &str) -> Option<(IStr, IStr)> {
        let var = IStr::new(var_name.to_string());
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
