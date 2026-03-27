use std::collections::{HashMap, HashSet};

use crate::ast::{
    Bind, BindValue, Expr, FileAst, FnCall, IfCondition, IfExpr, Loop, Pattern, WhenArm, WhenExpr,
};
use crate::intern::IStr;
use chumsky::span::SimpleSpan;
use crate::typeck::{FlowAnalysis, FlowContext, ImpossibleCheck, IndexOutOfBounds, TyEnv, TypeConstraint, Ty, LiteralValue};

/// Analyzes control flow to track type narrowing.
pub struct FlowAnalyzer<'a> {
    ty_env: &'a TyEnv,
    result: FlowAnalysis,
    /// Track variables that are reassigned (reset narrowing).
    reassigned: HashSet<IStr>,
    /// Stack of flow contexts for nested scopes.
    context_stack: Vec<FlowContext>,
    /// Current expression index for context mapping.
    expr_index: usize,
    /// Track which variables are in scope (parameters, let bindings).
    in_scope: HashSet<IStr>,
    /// Track types of local variables for bounds checking.
    locals: HashMap<IStr, Ty>,
}

impl<'a> FlowAnalyzer<'a> {
    pub fn new(ty_env: &'a TyEnv) -> Self {
        Self {
            ty_env,
            result: FlowAnalysis::new(),
            reassigned: HashSet::new(),
            context_stack: vec![FlowContext::new()],
            expr_index: 0,
            in_scope: HashSet::new(),
            locals: HashMap::new(),
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

    fn enter_bind(&mut self, bind: &Bind) {
        // Add parameters to scope
        if let Some(params) = bind.params().as_ref() {
            for (name, _) in params.iter() {
                self.in_scope.insert(*name);
            }
        }
        // Add self if this is a method
        if bind.receiver_type().is_some() {
            self.in_scope.insert(IStr::new("self".to_string()));
        }
    }

    fn analyze_bind(&mut self, bind: &Bind) {
        self.enter_bind(bind);

        match bind.value() {
            BindValue::Body { exprs, ret } => {
                for expr in exprs {
                    self.analyze_expr(expr);
                }
                if let Some(ret_expr) = &ret.0 {
                    self.analyze_expr(ret_expr);
                }
            }
            BindValue::Expr(expr) => {
                self.analyze_expr(expr);
            }
            BindValue::Extern => {}
        }
    }

    fn analyze_expr(&mut self, expr: &Expr) {
        let _index = self.expr_index;
        self.expr_index += 1;

        self.save_context_for_expr();

        match expr {
            // Pattern matching in if expressions.
            Expr::If(if_expr) => {
                self.analyze_if_expr(if_expr);
            }

            // Pattern matching in when expressions.
            Expr::When(when_expr) => {
                self.analyze_when_expr(when_expr);
            }

            // Variable binding (resets narrowing on reassignment).
            Expr::Bind(bind) => {
                // Add binding name to scope
                self.in_scope.insert(bind.name());

                // Mutable reassignment resets narrowing
                if !bind.is_const {
                    self.current_context_mut().reset(&bind.name());
                    self.reassigned.insert(bind.name());
                }

                // Track the type of the bound variable for bounds checking
                if let BindValue::Expr(expr) = bind.value() {
                    let ty = self.ty_env.infer_expr(expr, &self.locals);
                    self.locals.insert(bind.name(), ty);
                }

                self.analyze_bind(bind);
            }

            // Loops - conservative approach, don't persist narrowing across iterations.
            Expr::Loop(loop_expr) => {
                self.analyze_loop(loop_expr);
            }

            // Function call - analyze arguments.
            Expr::FnCall(call) => {
                self.analyze_fn_call(call);
            }

            // Binary operations.
            Expr::Binary(bin) => {
                self.analyze_expr(&bin.lhs);
                self.analyze_expr(&bin.rhs);
            }

            // Tuple operations.
            Expr::TupleLit(exprs) => {
                for expr in exprs {
                    self.analyze_expr(expr);
                }
            }
            Expr::TupleAlloc { init, .. } => {
                self.analyze_expr(init);
            }
            Expr::TupleGet { base, .. } => {
                self.analyze_expr(base);
            }
            Expr::TupleSet { base, value, .. } => {
                self.analyze_expr(base);
                self.analyze_expr(value);
            }

            // Buffer operations.
            Expr::BufGet { buf, index, span } => {
                self.analyze_expr(buf);
                self.analyze_expr(index);
                self.check_bounds(buf, index, *span);
            }
            Expr::BufSet { buf, index, value, span } => {
                self.analyze_expr(buf);
                self.analyze_expr(index);
                self.analyze_expr(value);
                self.check_bounds(buf, index, *span);
            }

            // Cast and reference operations.
            Expr::Cast { expr, .. } => {
                self.analyze_expr(expr);
            }
            Expr::TakePtr(inner) => {
                self.analyze_expr(inner);
            }
            Expr::TakeRef(inner) => {
                self.analyze_expr(inner);
            }
            Expr::Deref(inner) => {
                self.analyze_expr(inner);
            }
            Expr::Negate(inner) => {
                self.analyze_expr(inner);
            }

            // Tag operations.
            Expr::TagCall(_) => {}
            Expr::AnonymousTag(..) => {}
            Expr::SelfRef(_) => {}

            // Literals and other expressions.
            Expr::Lit(_) => {}
            Expr::FormatString(_) => {}
            Expr::Range(_) => {}
        }
    }

    fn analyze_if_expr(&mut self, if_expr: &IfExpr) {
        match &if_expr.condition {
            IfCondition::Bool(condition) => {
                self.analyze_expr(condition);
                // No narrowing for boolean conditions
                // Analyze body in a new scope
                self.push_context();
                for expr in &if_expr.body {
                    self.analyze_expr(expr);
                }
                self.pop_context();
            }
            IfCondition::Pattern { subject, tag } => {
                self.analyze_expr(subject);

                // Extract variable name from subject if it's a simple variable reference
                let var_name = self.extract_var_name(subject);

                if let Some(var) = var_name {
                    let variant_name = IStr::new(tag.name().to_string());

                    // Look up which union this variant belongs to
                    if let Some((union_name, _, _)) = self.ty_env.lookup_variant(variant_name) {
                        let constraint = TypeConstraint::IsVariant(union_name, variant_name);

                        // Check if this narrowing is impossible
                        if self.current_context().is_impossible(&var, &constraint) {
                            self.result.add_impossible_check(ImpossibleCheck {
                                expr_index: self.expr_index,
                                reason: format!(
                                    "Variable '{}' is already narrowed to a different variant",
                                    var.as_str()
                                ),
                            });
                        }

                        // Analyze body with narrowed context
                        self.push_context();
                        self.current_context_mut().narrow(var, constraint.clone());

                        // Every gin if block always ends with a `return` (syntax requirement),
                        // so any if block unconditionally exits the function early.
                        let has_early_return = true;
                        let _ = self.has_return_in_exprs(&if_expr.body); // kept for any body returns

                        for expr in &if_expr.body {
                            self.analyze_expr(expr);
                        }

                        let narrowed_context = self.context_stack.pop().unwrap();

                        // If the branch has an early return, merge the narrowing back to parent
                        if has_early_return {
                            self.merge_narrowing_from_branch(&narrowed_context);
                        }
                    } else {
                        // Unknown variant, analyze body without narrowing
                        self.push_context();
                        for expr in &if_expr.body {
                            self.analyze_expr(expr);
                        }
                        self.pop_context();
                    }
                } else {
                    // Complex subject, analyze body without narrowing
                    self.push_context();
                    for expr in &if_expr.body {
                        self.analyze_expr(expr);
                    }
                    self.pop_context();
                }
            }
        }
    }

    fn analyze_when_expr(&mut self, when_expr: &WhenExpr) {
        // If there's a subject, analyze it first
        if let Some(subject) = &when_expr.subject {
            self.analyze_expr(subject.as_ref());

            let var_name = self.extract_var_name(subject);

            // For pattern matching when, analyze each arm
            for arm in &when_expr.arms {
                match arm {
                    WhenArm::Is { pattern, body } => {
                        let variant_name = IStr::new(pattern.name().to_string());

                        if let Some((union_name, _, _)) = self.ty_env.lookup_variant(variant_name) {
                            let constraint = TypeConstraint::IsVariant(union_name, variant_name);

                            // Check for impossibility if we have a simple variable subject
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

                            // Analyze arm body with narrowed context
                            self.push_context();
                            if let Some(var) = var_name {
                                self.current_context_mut().narrow(var, constraint);
                            }
                            self.analyze_expr(body);
                            self.pop_context();
                        } else {
                            // Unknown variant, analyze without narrowing
                            self.push_context();
                            self.analyze_expr(body);
                            self.pop_context();
                        }
                    }
                    WhenArm::Cond { condition, body } => {
                        self.analyze_expr(condition);
                        self.push_context();
                        self.analyze_expr(body);
                        self.pop_context();
                    }
                    WhenArm::Else(body) => {
                        self.push_context();
                        self.analyze_expr(body);
                        self.pop_context();
                    }
                }
            }
        } else {
            // Boolean when - no narrowing
            for arm in &when_expr.arms {
                match arm {
                    WhenArm::Cond { condition, body } => {
                        self.analyze_expr(condition);
                        self.analyze_expr(body);
                    }
                    WhenArm::Else(body) => {
                        self.analyze_expr(body);
                    }
                    WhenArm::Is { .. } => {
                        // Is arm in boolean when - error case, but we still analyze
                    }
                }
            }
        }
    }

    fn analyze_loop(&mut self, loop_expr: &Loop) {
        match loop_expr {
            Loop::While(while_loop) => {
                self.analyze_expr(&while_loop.cond);

                // Save entry context
                let entry_context = self.current_context().clone();

                // Loop body may execute multiple times
                // Conservative: don't persist narrowing across iterations
                self.push_context();
                for expr in &while_loop.exprs {
                    self.analyze_expr(expr);
                }
                self.pop_context();

                // After loop, reset narrowing for variables modified in loop
                self.reset_loop_variables(&while_loop.exprs, entry_context);
            }
            Loop::ForIn(for_loop) => {
                // Analyze iterator
                self.analyze_expr(&for_loop.iter);

                let entry_context = self.current_context().clone();

                // Add loop variable to scope (from pattern)
                if let Pattern::Ident(var_name) = &for_loop.pat {
                    self.in_scope.insert(*var_name);
                }

                self.push_context();
                for expr in &for_loop.exprs {
                    self.analyze_expr(expr);
                }
                self.pop_context();

                // Remove loop variable from scope
                if let Pattern::Ident(var_name) = &for_loop.pat {
                    self.in_scope.remove(var_name);
                }

                // Reset narrowing for variables modified in loop
                self.reset_loop_variables(&for_loop.exprs, entry_context);
            }
        }
    }

    fn analyze_fn_call(&mut self, call: &FnCall) {
        if let Some(args) = &call.args {
            for arg in args {
                self.analyze_expr(arg);
            }
        }
    }

    /// Check if a buffer access is out of bounds at compile time.
    ///
    /// Only catches literal indices on arrays with known sizes.
    /// For runtime bounds checking with type narrowing, this is a TODO.
    fn check_bounds(&mut self, buf: &Expr, index: &Expr, span: SimpleSpan) {
        use crate::typeck::infer_expr_ty;

        let index_val = match infer_expr_ty(index, &self.locals, &self.ty_env.tag_types, &self.ty_env.fn_return_types) {
            Ty::Literal(LiteralValue::Int(n)) => n,
            Ty::Int(_) => return,
            _ => return,
        };

        let buf_size = match infer_expr_ty(buf, &self.locals, &self.ty_env.tag_types, &self.ty_env.fn_return_types) {
            Ty::Array { size, .. } => size,
            _ => return,
        };

        if index_val < 0 || index_val as usize >= buf_size {
            self.result.add_bounds_check(IndexOutOfBounds {
                expr_index: self.expr_index,
                span,
                index: index_val,
                size: buf_size,
            });
        }
    }

    fn extract_var_name(&self, expr: &Expr) -> Option<IStr> {
        match expr {
            Expr::AnonymousTag(name, _) if self.in_scope.contains(name) => Some(*name),
            // Lowercase variable references are parsed as FnCall with no args and no path segments.
            Expr::FnCall(call) if call.args.is_none() && call.path.segments.is_empty() => {
                let name = call.path.root;
                if self.in_scope.contains(&name) {
                    Some(name)
                } else {
                    None
                }
            }
            Expr::SelfRef(_) => Some(IStr::new("self".to_string())),
            _ => None,
        }
    }

    fn has_return_in_exprs(&self, exprs: &[Expr]) -> bool {
        exprs.iter().any(|e| self.has_return(e))
    }

    fn has_return(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Loop(loop_expr) => match loop_expr {
                Loop::While(w) => self.has_return_in_exprs(&w.exprs),
                Loop::ForIn(f) => self.has_return_in_exprs(&f.exprs),
            },
            // Every gin if block always ends with a `return` (it's part of the syntax),
            // so any IfExpr is always an early return from the enclosing function.
            Expr::If(_) => true,
            Expr::Bind(bind) => {
                if let BindValue::Body { exprs, ret } = bind.value() {
                    self.has_return_in_exprs(exprs)
                        || ret.0.as_ref().is_some_and(|e| self.has_return(e))
                } else if let BindValue::Expr(e) = bind.value() {
                    self.has_return(e)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn merge_narrowing_from_branch(&mut self, branch_ctx: &FlowContext) {
        // After an early-returning branch that required IsVariant(u, v),
        // code past that branch knows the variable is NOT that variant.
        for (var, constraint) in branch_ctx.local_constraints() {
            if !self.current_context().has_local_constraint(var) {
                let negated = match constraint {
                    TypeConstraint::IsVariant(u, v) => TypeConstraint::IsNotVariant(*u, *v),
                    TypeConstraint::IsNotVariant(u, v) => TypeConstraint::IsVariant(*u, *v),
                };
                self.current_context_mut().narrow(*var, negated);
            }
        }
    }

    fn reset_loop_variables(&mut self, exprs: &[Expr], entry_context: FlowContext) {
        // Reset narrowing for variables that are assigned in the loop
        for expr in exprs {
            if let Expr::Bind(bind) = expr {
                self.current_context_mut().reset(&bind.name());
            }
        }
        // Restore variables that weren't modified in the loop
        let in_scope = self.in_scope.clone();
        for var in in_scope.iter() {
            if !self.reassigned.contains(var)
                && let Some(entry_constraint) = entry_context.get_constraint(var) {
                    self.current_context_mut()
                        .narrow(*var, entry_constraint.clone());
                }
        }
    }

    pub fn into_result(self) -> FlowAnalysis {
        let mut result = self.result;
        result.final_context = self.context_stack.into_iter().next().unwrap_or_default();
        result.union_to_variants = self.ty_env.build_union_to_variants();
        result
    }
}
