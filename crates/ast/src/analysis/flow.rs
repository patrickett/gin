use std::collections::{HashMap, HashSet};

use diagnostic::{Category, Diagnostic, DiagnosticCode, TypeSymptom};
use internment::Intern;

use crate::analysis::const_value::{Bound, ConstValue, TypeConstraint};
use crate::analysis::resolve::is_type_surface;
use crate::span::{HasSpanId, SpanId};
use crate::ty::Ty;
use crate::{
    Bind, BindValue, Expr, FileAst, FnCall, IfCondition, IfExpr, Loop, TypeExpr, Typed, WhenArm,
    WhenExpr, for_loop_pattern_names, pattern_type_binding_names, type_surface_mangle_name,
};
use crate::{TyInfer, TyInferEnv};

/// The state of a variable during flow analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VarState {
    /// Variable is alive and usable.
    Alive,
    /// Variable has been consumed (ownership transferred).
    Consumed,
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

    pub fn get_context(&self, index: usize) -> Option<&FlowContext> {
        self.expr_contexts.get(&index)
    }

    pub fn insert_context(&mut self, index: usize, ctx: FlowContext) {
        self.expr_contexts.insert(index, ctx);
    }

    pub fn insert_span(&mut self, span_id: SpanId, index: usize) {
        self.expr_spans.insert(span_id, index);
    }

    pub fn narrowed_at(&self, expr_index: usize, var_name: &str) -> Option<&TypeConstraint> {
        let ctx = self.expr_contexts.get(&expr_index)?;
        let var = Intern::<String>::from_ref(var_name);
        ctx.get_constraint(&var)
    }

    pub fn value_at(&self, expr_index: usize, var_name: &str) -> Option<&ConstValue> {
        let ctx = self.expr_contexts.get(&expr_index)?;
        let var = Intern::<String>::from_ref(var_name);
        ctx.get_constant(&var)
    }

    pub fn add_impossible_check(&mut self, check: ImpossibleCheck) {
        self.impossible_checks.push(check);
    }

    pub fn add_bounds_check(&mut self, check: IndexOutOfBounds) {
        self.bounds_checks.push(check);
    }

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

/// Analyzes control flow to track type narrowing.
pub struct FlowAnalyzer<'a> {
    tag_types: &'a HashMap<Intern<String>, Ty>,
    fn_return_types: &'a HashMap<Intern<String>, Ty>,
    variant_map: &'a crate::VariantMap,
    result: FlowAnalysis,
    /// Track variables that are reassigned (reset narrowing).
    reassigned: HashSet<Intern<String>>,
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
    pub fn new(
        tag_types: &'a HashMap<Intern<String>, Ty>,
        fn_return_types: &'a HashMap<Intern<String>, Ty>,
        variant_map: &'a crate::VariantMap,
    ) -> Self {
        Self {
            tag_types,
            fn_return_types,
            variant_map,
            result: FlowAnalysis::new(),
            reassigned: HashSet::new(),
            context_stack: vec![FlowContext::new()],
            expr_index: 0,
            in_scope: HashSet::new(),
            locals: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn analyze_file(&mut self, ast: &FileAst) {
        for bind in ast.defs.values() {
            if !bind.attributes().matches_current_platform() {
                continue;
            }
            self.analyze_bind(bind);
        }
    }

    fn current_context(&self) -> &FlowContext {
        self.context_stack.last().unwrap()
    }

    fn current_context_mut(&mut self) -> &mut FlowContext {
        self.context_stack.last_mut().unwrap()
    }

    fn push_context(&mut self) {
        let parent = self.current_context().clone();
        self.context_stack.push(FlowContext::with_parent(parent));
    }

    fn pop_context(&mut self) {
        self.context_stack.pop();
    }

    fn save_context_for_expr(&mut self) {
        self.result
            .insert_context(self.expr_index, self.current_context().clone());
    }

    fn analyze_spanned_expr(&mut self, expr: &Typed<Expr>) {
        self.result.expr_spans.insert(expr.span_id, self.expr_index);
        self.analyze_expr(expr);
    }

    fn enter_bind(&mut self, bind: &Bind) {
        if let Some(params) = bind.params().as_ref() {
            for (name, _) in params.iter() {
                self.in_scope.insert(*name);
                self.current_context_mut()
                    .set_var_state(*name, VarState::Alive);
            }
        }
        if bind.is_method() {
            let self_name = Intern::<String>::from_ref("self");
            self.in_scope.insert(self_name);
            self.current_context_mut()
                .set_var_state(self_name, VarState::Alive);
        }
    }

    fn analyze_bind(&mut self, bind: &Bind) {
        self.enter_bind(bind);

        self.current_context_mut()
            .set_var_state(bind.name(), VarState::Alive);

        match bind.value() {
            BindValue::Body { exprs, ret } => {
                for expr in exprs {
                    self.analyze_spanned_expr(expr);
                }
                if let Some(ret_expr) = &ret.value {
                    self.analyze_spanned_expr(ret_expr);
                }
            }
            BindValue::Expr(expr) => {
                self.analyze_spanned_expr(expr);
            }
            BindValue::Extern => {}
        }
    }

    fn analyze_expr(&mut self, expr: &Expr) {
        self.save_context_for_expr();
        self.expr_index += 1;

        match expr {
            Expr::If(if_expr) => {
                self.analyze_if_expr(if_expr);
            }

            Expr::When(when_expr) => {
                self.analyze_when_expr(when_expr);
            }

            Expr::Bind(bind) => {
                self.in_scope.insert(bind.name());

                self.current_context_mut().reset(&bind.name());
                self.current_context_mut()
                    .set_var_state(bind.name(), VarState::Alive);
                self.reassigned.insert(bind.name());

                if let BindValue::Expr(expr) = bind.value() {
                    let env = TyInferEnv {
                        tag_types: self.tag_types,
                        fn_return_types: self.fn_return_types,
                        locals: &self.locals,
                        tag_params: None,
                    };
                    let ty = expr.infer_ty(&env);
                    self.locals.insert(bind.name(), ty);
                }

                self.analyze_bind(bind);
            }

            Expr::Loop(loop_expr) => {
                self.analyze_loop(loop_expr);
            }

            Expr::FnCall(call) => {
                if call.args.is_none() && call.path.segments.is_empty() {
                    let var_name = call.path.root;
                    if self.in_scope.contains(&var_name) {
                        let ctx = self.current_context();
                        if let Some(VarState::Consumed) = ctx.get_var_state(&var_name) {
                            self.diagnostics.push(Diagnostic {
                                code: DiagnosticCode::Type(TypeSymptom::UseOfMovedValue {
                                    name: var_name.as_str().to_string(),
                                }),
                                message: format!("use of moved value `{}`", var_name.as_str()),
                                help_on_span: None,
                                help: Some("value was moved into another owner".into()),
                                span_id: crate::span::SpanId::INVALID,
                                category: Category::Flaw,
                                related: Vec::new(),
                            });
                        }
                    }
                }
                self.analyze_fn_call(call);
            }

            Expr::Binary(bin) => {
                self.analyze_spanned_expr(&bin.lhs);
                self.analyze_spanned_expr(&bin.rhs);
            }

            Expr::TupleLit(exprs) | Expr::List(exprs) => {
                for expr in exprs {
                    self.analyze_spanned_expr(expr);
                }
            }
            Expr::TupleAlloc { init, .. } => {
                self.analyze_spanned_expr(init);
            }
            Expr::TupleGet { base, .. } => {
                self.analyze_spanned_expr(base);
            }
            Expr::TupleSet { base, value, .. } => {
                self.analyze_spanned_expr(base);
                self.analyze_spanned_expr(value);
            }

            Expr::BufGet { buf, index } => {
                self.analyze_spanned_expr(buf);
                self.analyze_spanned_expr(index);
            }
            Expr::BufSet { buf, index, value } => {
                self.analyze_spanned_expr(buf);
                self.analyze_spanned_expr(index);
                self.analyze_spanned_expr(value);
            }

            Expr::Cast { expr, .. } => {
                self.analyze_spanned_expr(expr);
            }
            Expr::TakePtr(inner) => {
                self.analyze_spanned_expr(inner);
            }
            Expr::Ref { inner, .. } => {
                self.analyze_spanned_expr(inner);
            }
            Expr::ConsumeArg(inner) => {
                self.analyze_spanned_expr(inner);
            }
            Expr::Eat(inner) => {
                // eat expr: analyze and mark the inner as consumed.
                // If it's a bare variable, mark it consumed in the flow context.
                if let Expr::FnCall(FnCall { path, args: None }) = &inner.value {
                    self.current_context_mut()
                        .set_var_state(path.root, VarState::Consumed);
                }
                self.analyze_spanned_expr(inner);
            }
            Expr::Deref(inner) => {
                self.analyze_spanned_expr(inner);
            }
            Expr::Negate(inner) => {
                self.analyze_spanned_expr(inner);
            }

            Expr::TagCall(_) => {}
            Expr::AnonymousTag(..) => {}
            Expr::SelfRef => {}

            Expr::Lit(_) => {}
            Expr::FormatString(_) => {}
            Expr::Range(_) => {}
            Expr::Asm(_) => {}
            Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. } => {}
        }
    }

    fn analyze_if_expr(&mut self, if_expr: &IfExpr) {
        match &if_expr.condition {
            IfCondition::Bool(condition) => {
                self.analyze_spanned_expr(condition);

                let cond_comparisons = self.extract_comparisons(&condition.value);

                self.push_context();
                for (var, constraint) in &cond_comparisons {
                    self.current_context_mut().narrow(*var, constraint.clone());
                }
                for expr in &if_expr.body {
                    self.analyze_spanned_expr(expr);
                }

                let narrowed_context = self.context_stack.pop().unwrap();
                self.merge_narrowing_from_branch(&narrowed_context);
            }
            IfCondition::Pattern { subject, pattern } => {
                self.analyze_spanned_expr(subject);

                if !is_type_surface(&pattern.value) {
                    self.push_context();
                    for expr in &if_expr.body {
                        self.analyze_spanned_expr(expr);
                    }
                    self.pop_context();
                    return;
                }

                let var_name = self.extract_var_name(subject);

                if let Some(var) = var_name {
                    let variant_name =
                        Intern::<String>::from_ref(type_surface_mangle_name(&pattern.value));

                    if let Some(entry) = self.variant_map.get(&variant_name)
                        && let Some((union_name, _, _)) = entry.first()
                    {
                        let constraint = TypeConstraint::IsVariant(*union_name, variant_name);

                        if self.current_context().is_impossible(&var, &constraint) {
                            self.result.add_impossible_check(ImpossibleCheck {
                                expr_index: self.expr_index,
                                reason: format!(
                                    "Variable '{}' is already narrowed to a different variant",
                                    var.as_str()
                                ),
                            });
                        }

                        self.push_context();
                        self.current_context_mut().narrow(var, constraint.clone());

                        let pat_bindings = self.push_pattern_bindings(&pattern.value);

                        for expr in &if_expr.body {
                            self.analyze_spanned_expr(expr);
                        }

                        self.pop_tag_pattern_bindings(&pat_bindings);

                        let narrowed_context = self.context_stack.pop().unwrap();
                        self.merge_narrowing_from_branch(&narrowed_context);
                    } else {
                        self.push_context();
                        let pat_bindings = self.push_pattern_bindings(&pattern.value);
                        for expr in &if_expr.body {
                            self.analyze_spanned_expr(expr);
                        }
                        self.pop_tag_pattern_bindings(&pat_bindings);
                        self.pop_context();
                    }
                } else {
                    self.push_context();
                    let pat_bindings = self.push_pattern_bindings(&pattern.value);
                    for expr in &if_expr.body {
                        self.analyze_spanned_expr(expr);
                    }
                    self.pop_tag_pattern_bindings(&pat_bindings);
                    self.pop_context();
                }
            }
        }
    }

    fn analyze_when_expr(&mut self, when_expr: &WhenExpr) {
        if let Some(subject) = &when_expr.subject {
            self.analyze_spanned_expr(subject.as_ref());

            let var_name = self.extract_var_name(subject);

            for arm in &when_expr.arms {
                match arm {
                    WhenArm::Is { pattern, body, .. } => {
                        if !is_type_surface(&pattern.value) {
                            self.push_context();
                            self.analyze_spanned_expr(body);
                            self.pop_context();
                            continue;
                        }
                        let variant_name =
                            Intern::<String>::from_ref(type_surface_mangle_name(&pattern.value));

                        if let Some(entry) = self.variant_map.get(&variant_name)
                            && let Some((union_name, _, _)) = entry.first()
                        {
                            let constraint = TypeConstraint::IsVariant(*union_name, variant_name);

                            if let Some(var) = var_name
                                && self.current_context().is_impossible(&var, &constraint)
                            {
                                self.result.add_impossible_check(ImpossibleCheck {
                                    expr_index: self.expr_index,
                                    reason: format!(
                                        "Variant '{}' is impossible given previous narrowing",
                                        variant_name.as_str()
                                    ),
                                });
                            }

                            self.push_context();
                            if let Some(var) = var_name {
                                self.current_context_mut().narrow(var, constraint);
                            }
                            let pat_bindings = self.push_pattern_bindings(&pattern.value);
                            self.analyze_spanned_expr(body);
                            self.pop_tag_pattern_bindings(&pat_bindings);
                            self.pop_context();
                        } else {
                            self.push_context();
                            let pat_bindings = self.push_pattern_bindings(&pattern.value);
                            self.analyze_spanned_expr(body);
                            self.pop_tag_pattern_bindings(&pat_bindings);
                            self.pop_context();
                        }
                    }
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        self.analyze_spanned_expr(condition);
                        self.push_context();
                        self.analyze_spanned_expr(body);
                        self.pop_context();
                    }
                    WhenArm::Else(body, _) => {
                        self.push_context();
                        self.analyze_spanned_expr(body);
                        self.pop_context();
                    }
                }
            }
        } else {
            for arm in &when_expr.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        self.analyze_spanned_expr(condition);
                        self.analyze_spanned_expr(body);
                    }
                    WhenArm::Else(body, _) => {
                        self.analyze_spanned_expr(body);
                    }
                    WhenArm::Is { .. } => {}
                }
            }
        }
    }

    fn analyze_loop(&mut self, loop_expr: &Loop) {
        match loop_expr {
            Loop::While(while_loop) => {
                self.analyze_spanned_expr(&while_loop.cond);

                let cond_comparisons = self.extract_comparisons(&while_loop.cond.value);

                let entry_context = self.current_context().clone();

                self.push_context();
                for (var, constraint) in &cond_comparisons {
                    self.current_context_mut().narrow(*var, constraint.clone());
                }
                for expr in &while_loop.exprs {
                    self.analyze_spanned_expr(expr);
                }
                self.pop_context();

                self.reset_loop_variables(&while_loop.exprs, entry_context);

                for (var, constraint) in &cond_comparisons {
                    let negated = constraint.negate();
                    self.current_context_mut().narrow(*var, negated);
                }
            }
            Loop::ForIn(for_loop) => {
                self.analyze_spanned_expr(&for_loop.iter);

                let entry_context = self.current_context().clone();

                if let Some(names) = for_loop_pattern_names(&for_loop.pat.value) {
                    for n in &names {
                        self.in_scope.insert(*n);
                    }
                }

                self.push_context();
                for expr in &for_loop.exprs {
                    self.analyze_spanned_expr(expr);
                }
                self.pop_context();

                if let Some(names) = for_loop_pattern_names(&for_loop.pat.value) {
                    for n in &names {
                        self.in_scope.remove(n);
                    }
                }

                self.reset_loop_variables(&for_loop.exprs, entry_context);
            }
        }
    }

    fn analyze_fn_call(&mut self, call: &FnCall) {
        if let Some(args) = &call.args {
            for arg in args {
                self.analyze_spanned_expr(arg);
            }
        }
    }

    fn extract_var_name(&self, expr: &Expr) -> Option<Intern<String>> {
        match expr {
            Expr::AnonymousTag(name) if self.in_scope.contains(name) => Some(*name),
            Expr::FnCall(call) if call.args.is_none() && call.path.segments.is_empty() => {
                let name = call.path.root;
                if self.in_scope.contains(&name) {
                    Some(name)
                } else {
                    None
                }
            }
            Expr::SelfRef => Some(Intern::<String>::from_ref("self")),
            _ => None,
        }
    }

    fn extract_comparisons(&self, expr: &Expr) -> Vec<(Intern<String>, TypeConstraint)> {
        match expr {
            Expr::Binary(bin) => {
                if bin.op.is_comparison() {
                    let var = match self.extract_var_name(&bin.lhs.value) {
                        Some(v) if self.in_scope.contains(&v) => v,
                        _ => return Vec::new(),
                    };
                    let bound = if let Some(rhs_var) = self.extract_var_name(&bin.rhs.value) {
                        Bound::Variable(rhs_var)
                    } else if let Some(val) = self.eval_const(&bin.rhs.value) {
                        Bound::Constant(val)
                    } else {
                        return Vec::new();
                    };
                    return vec![(
                        var,
                        TypeConstraint::Compare {
                            op: bin.op.clone(),
                            bound,
                        },
                    )];
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn eval_const(&self, expr: &Expr) -> Option<ConstValue> {
        match expr {
            Expr::Lit(lit) => match lit {
                crate::Literal::Int(n) => Some(ConstValue::Int(*n as i128)),
                crate::Literal::Float(f) => Some(ConstValue::Float(*f)),
                crate::Literal::String(s) => Some(ConstValue::String(s.clone())),
                crate::Literal::Number(n) => Some(ConstValue::Int(*n as i128)),
            },
            Expr::TagCall(call) => {
                let mut args = Vec::new();
                let mut named_fields: Vec<(Intern<String>, ConstValue)> = Vec::new();
                let mut all_named = true;
                for arg in &call.args {
                    match &arg.value {
                        Expr::Bind(bind) => {
                            if let crate::BindValue::Expr(sp) = bind.value() {
                                if let Some(val) = self.eval_const(&sp.value) {
                                    named_fields.push((bind.name(), val));
                                } else {
                                    all_named = false;
                                }
                            } else {
                                all_named = false;
                            }
                        }
                        _ => {
                            all_named = false;
                            args.push(self.eval_const(&arg.value)?);
                        }
                    }
                }
                if all_named && !named_fields.is_empty() {
                    Some(ConstValue::Record {
                        fields: named_fields,
                    })
                } else {
                    let qual_path = call.qual_path.as_ref().map(|p| {
                        let mut s = p.root.as_str().to_string();
                        for seg in &p.segments {
                            s.push('.');
                            s.push_str(seg.as_str());
                        }
                        s
                    });
                    Some(ConstValue::Tag {
                        name: call.name,
                        qual_path,
                        args,
                    })
                }
            }
            Expr::Bind(bind) if bind.params().is_none() => {
                if let crate::BindValue::Expr(sp) = bind.value() {
                    self.eval_const(&sp.value)
                } else {
                    None
                }
            }
            Expr::TupleLit(elems) => {
                let mut items = Vec::with_capacity(elems.len());
                for elem in elems {
                    items.push(self.eval_const(&elem.value)?);
                }
                Some(ConstValue::List(items))
            }
            Expr::Binary(bin) => {
                let lhs = self.eval_const(&bin.lhs.value)?;
                let rhs = self.eval_const(&bin.rhs.value)?;
                lhs.eval_binop(&bin.op, &rhs)
            }
            Expr::FnCall(call) if call.args.is_none() && call.path.segments.is_empty() => self
                .current_context()
                .get_constant(&call.path.root)
                .cloned(),
            _ => None,
        }
    }

    fn push_pattern_bindings(&mut self, pat: &TypeExpr) -> Vec<Intern<String>> {
        let names = pattern_type_binding_names(pat);
        for n in &names {
            self.in_scope.insert(*n);
        }
        names
    }

    fn pop_tag_pattern_bindings(&mut self, names: &[Intern<String>]) {
        for n in names {
            self.in_scope.remove(n);
        }
    }

    fn merge_narrowing_from_branch(&mut self, branch_ctx: &FlowContext) {
        for (var, constraint) in branch_ctx.local_constraints() {
            if !self.current_context().has_local_constraint(var) {
                let negated = constraint.negate();
                self.current_context_mut().narrow(*var, negated);
                self.current_context_mut().reset_constant(var);
            }
        }
    }

    fn reset_loop_variables(&mut self, exprs: &[Typed<Expr>], entry_context: FlowContext) {
        for expr in exprs {
            if let Expr::Bind(bind) = &**expr {
                self.current_context_mut().reset(&bind.name());
            }
        }
        let in_scope = self.in_scope.clone();
        for var in in_scope.iter() {
            if !self.reassigned.contains(var)
                && let Some(entry_constraint) = entry_context.get_constraint(var)
            {
                self.current_context_mut()
                    .narrow(*var, entry_constraint.clone());
            }
        }
    }

    pub fn into_result(self) -> FlowAnalysis {
        let mut result = self.result;
        result.final_context = self.context_stack.into_iter().next().unwrap_or_default();
        result.union_to_variants = {
            let mut map: HashMap<Intern<String>, Vec<Intern<String>>> = HashMap::new();
            for (variant_name, entries) in self.variant_map.iter() {
                for (union_name, _, _) in entries {
                    map.entry(*union_name).or_default().push(*variant_name);
                }
            }
            map
        };
        result
    }
}
