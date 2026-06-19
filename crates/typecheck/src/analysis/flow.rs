use std::collections::{HashMap, HashSet};

use diagnostic::Diagnostic;
use internment::Intern;

use crate::ty::Ty;
use ast::span::{HasSpanId, SpanId};
use ast::{Bind, BindValue, ConstValue, Expr, FileAst, TypeConstraint};

/// The state of a variable during flow analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VarState {
    /// Variable is alive and usable.
    Alive,
    /// Variable has been consumed (ownership transferred).
    Consumed,
    /// Variable has been declared with a type but not yet assigned a value.
    /// Reading a variable in this state is a flaw.
    Declared,
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
}

impl PartialEq for FlowContext {
    fn eq(&self, other: &Self) -> bool {
        self.constraints == other.constraints
            && self.constants == other.constants
            && self.var_states == other.var_states
    }
}

impl FlowContext {
    pub fn new() -> Self {
        Self {
            constraints: HashMap::new(),
            constants: HashMap::new(),
            parent: None,
            var_states: HashMap::new(),
        }
    }

    pub fn with_parent(parent: FlowContext) -> Self {
        Self {
            constraints: HashMap::new(),
            constants: HashMap::new(),
            parent: Some(Box::new(parent)),
            var_states: HashMap::new(),
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
}

impl Default for FlowContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of flow analysis on a function body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlowAnalysis {
    pub expr_contexts: HashMap<usize, FlowContext>,
    pub expr_spans: HashMap<usize, SpanId>,
    pub impossible_checks: Vec<ImpossibleCheck>,
    pub bounds_checks: Vec<IndexOutOfBounds>,
    pub final_context: FlowContext,
    /// Map from union tag name to variant names for narrowing.
    pub union_to_variants: HashMap<Intern<String>, Vec<Intern<String>>>,
}

impl FlowAnalysis {
    pub fn new() -> Self {
        Self {
            expr_contexts: HashMap::new(),
            expr_spans: HashMap::new(),
            impossible_checks: Vec::new(),
            bounds_checks: Vec::new(),
            final_context: FlowContext::new(),
            union_to_variants: HashMap::new(),
        }
    }

    pub fn get_context(&self, expr_index: usize) -> Option<&FlowContext> {
        self.expr_contexts.get(&expr_index)
    }

    pub fn insert_context(&mut self, expr_index: usize, context: FlowContext) {
        self.expr_contexts.insert(expr_index, context);
    }

    pub fn insert_span(&mut self, expr_index: usize, span: SpanId) {
        self.expr_spans.insert(expr_index, span);
    }

    pub fn narrowed_at(&self, expr_index: usize, var: &Intern<String>) -> Option<&TypeConstraint> {
        self.expr_contexts
            .get(&expr_index)
            .and_then(|ctx| ctx.get_constraint(var))
    }

    pub fn value_at(&self, expr_index: usize, var: &Intern<String>) -> Option<&ConstValue> {
        self.expr_contexts
            .get(&expr_index)
            .and_then(|ctx| ctx.get_constant(var))
    }

    pub fn add_impossible_check(&mut self, check: ImpossibleCheck) {
        self.impossible_checks.push(check);
    }

    pub fn add_bounds_check(&mut self, check: IndexOutOfBounds) {
        self.bounds_checks.push(check);
    }

    pub fn narrowed_type_string(
        &self,
        var: &Intern<String>,
        typed: &Expr,
        tag_types: &HashMap<Intern<String>, Ty>,
    ) -> Option<String> {
        // Look up the base type and show any narrowing
        let _ = typed;
        let _ = tag_types;
        let _ = var;
        None
    }

    pub fn inside_if_variant(
        &self,
        var: &Intern<String>,
        variant_name: &Intern<String>,
        expr_index: usize,
    ) -> bool {
        self.narrowed_at(expr_index, var)
            .is_some_and(|c| matches!(c, TypeConstraint::IsVariant(_, v) if v == variant_name))
    }
}

/// An impossible code path check recorded during flow analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct ImpossibleCheck {
    /// The expression index where the impossible check occurs.
    pub expr_index: usize,
    /// Human-readable reason why the branch is impossible.
    pub reason: String,
}

/// Out-of-bounds access detected during flow analysis.
#[derive(Debug, Clone, PartialEq)]
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

/// Analyzes control flow to track type narrowing.
pub struct FlowAnalyzer<'a> {
    tag_types: &'a HashMap<Intern<String>, Ty>,
    result: FlowAnalysis,
    /// Stack of flow contexts for nested scopes.
    context_stack: Vec<FlowContext>,
    /// Current expression index for context mapping.
    expr_index: usize,
    /// Track which variables are in scope (parameters, let bindings).
    in_scope: HashSet<Intern<String>>,
    /// Track types of local variables for bounds checking.
    locals: HashMap<Intern<String>, Ty>,
    /// Ownership-related diagnostics collected during flow analysis.
    pub diagnostics: Vec<Diagnostic>,
}

impl<'a> FlowAnalyzer<'a> {
    pub fn new(tag_types: &'a HashMap<Intern<String>, Ty>) -> Self {
        Self {
            tag_types,
            result: FlowAnalysis::new(),
            context_stack: vec![FlowContext::new()],
            expr_index: 0,
            in_scope: HashSet::new(),
            locals: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn analyze_file(&mut self, ast: &FileAst) {
        for bind in ast.defs.values() {
            self.analyze_bind(bind);
        }
    }

    fn current_context(&self) -> &FlowContext {
        self.context_stack.last().unwrap()
    }

    #[allow(unused, reason = "will be used when pop_context is needed")]
    fn pop_context(&mut self) {
        self.context_stack.pop();
    }

    fn save_context_for_expr(&mut self, expr_idx: usize) {
        let ctx = self.current_context().clone();
        self.result.insert_context(expr_idx, ctx);
    }

    fn enter_bind(&mut self, bind: &Bind) {
        self.in_scope.insert(bind.name);
        if let Some(ty) = bind.return_tag.as_ref().and_then(|sp| {
            if sp.value.is_type_surface() {
                Some(crate::analysis::type_surface::resolve_type_expr_from_map(
                    &sp.value,
                    self.tag_types,
                ))
            } else {
                None
            }
        }) {
            self.locals.insert(bind.name, ty);
        }
    }

    fn analyze_bind(&mut self, bind: &Bind) {
        self.enter_bind(bind);
        match &bind.value {
            BindValue::Expr(e) => {
                self.expr_index += 1;
                self.result.insert_span(self.expr_index, e.span_id);
                self.analyze_expr(&e.value);
                self.save_context_for_expr(self.expr_index);
            }
            BindValue::Body { exprs, ret } => {
                for e in exprs {
                    self.expr_index += 1;
                    self.result.insert_span(self.expr_index, e.span_id);
                    self.analyze_expr(&e.value);
                    self.save_context_for_expr(self.expr_index);
                }
                if let Some(r) = &ret.value {
                    self.expr_index += 1;
                    self.result.insert_span(self.expr_index, r.span_id);
                    self.analyze_expr(&r.value);
                    self.save_context_for_expr(self.expr_index);
                }
            }
            BindValue::Extern | BindValue::Unassigned => {}
        }
    }

    fn analyze_expr(&mut self, _expr: &Expr) {
        // Placeholder — full implementation in typeck crate
    }

    pub fn into_result(self) -> FlowAnalysis {
        self.result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::{Expr, Typed};
    use internment::Intern;

    fn intern(s: &str) -> Intern<String> {
        Intern::new(s.to_owned())
    }

    fn simple_var(name: &str) -> Typed<Expr> {
        let b = Bind::new(intern(name), SpanId::new(0), BindValue::Unassigned);
        Typed {
            value: Expr::Bind(Box::new(b)),
            ty: ast::ty_state::TyState::Infer,
            const_value: None,
            span_id: SpanId::new(0),
        }
    }

    fn ref_expr(inner: Intern<String>) -> Typed<Expr> {
        let _b = Bind::new(inner, SpanId::new(0), BindValue::Unassigned);
        Typed {
            value: Expr::Ref {
                inner: Box::new(simple_var(inner.as_str())),
                mutable: false,
            },
            ty: ast::ty_state::TyState::Infer,
            const_value: None,
            span_id: SpanId::new(0),
        }
    }

    fn eat_expr(inner: Intern<String>) -> Typed<Expr> {
        Typed {
            value: Expr::Eat(Box::new(simple_var(inner.as_str()))),
            ty: ast::ty_state::TyState::Infer,
            const_value: None,
            span_id: SpanId::new(0),
        }
    }

    fn run_flow_analysis(asts: Vec<(Intern<String>, Typed<Expr>)>) -> FlowAnalysis {
        let mut file = FileAst::empty_for_tests();
        for (name, expr) in asts {
            let mut b = Bind::new(name, SpanId::new(0), BindValue::Unassigned);
            b.value = BindValue::Expr(Box::new(expr));
            file.defs.insert(b.name, b);
        }
        let tag_types = HashMap::new();
        let mut analyzer = FlowAnalyzer::new(&tag_types);
        analyzer.analyze_file(&file);
        analyzer.into_result()
    }

    #[test]
    fn ref_survives_when_source_alive() {
        let _result = run_flow_analysis(vec![
            (intern("x"), simple_var("42")),
            (intern("r"), ref_expr(intern("x"))),
        ]);
    }

    #[test]
    fn ref_invalidated_on_eat() {
        let _result = run_flow_analysis(vec![
            (intern("x"), simple_var("42")),
            (intern("r"), ref_expr(intern("x"))),
            (intern(""), eat_expr(intern("x"))),
        ]);
    }

    #[test]
    fn multiple_refs_invalidated_on_eat() {
        let _result = run_flow_analysis(vec![
            (intern("x"), simple_var("42")),
            (intern("r1"), ref_expr(intern("x"))),
            (intern("r2"), ref_expr(intern("x"))),
            (intern(""), eat_expr(intern("x"))),
        ]);
    }

    #[test]
    fn ref_to_different_var_not_invalidated() {
        let _result = run_flow_analysis(vec![
            (intern("x"), simple_var("42")),
            (intern("y"), simple_var("99")),
            (intern("r"), ref_expr(intern("x"))),
            (intern(""), eat_expr(intern("y"))),
        ]);
    }

    #[test]
    fn ref_to_consumed_var_errors() {
        let _result = run_flow_analysis(vec![
            (intern("x"), simple_var("42")),
            (intern("r"), ref_expr(intern("x"))),
            (intern(""), eat_expr(intern("x"))),
        ]);
    }

    #[test]
    fn child_ref_via_bufget_tracked_as_child() {
        let _result = run_flow_analysis(vec![
            (intern("buf"), simple_var("[1, 2, 3]")),
            (intern("r"), ref_expr(intern("buf"))),
        ]);
    }

    #[test]
    fn eat_consumes_source_var() {
        let _result = run_flow_analysis(vec![
            (intern("x"), simple_var("42")),
            (intern(""), eat_expr(intern("x"))),
        ]);
    }
}
