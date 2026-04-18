use crate::TyInfer;
use crate::flow::{
    Bound, CmpOp, ConstValue, FlowAnalysis, FlowContext, ImpossibleCheck, IndexOutOfBounds,
    TypeConstraint,
};
use crate::r#type::{Ty, TyEnv};
use ast::SpanId;
use ast::Spanned;
use ast::{
    Bind, BindValue, Expr, FileAst, FnCall, IfCondition, IfExpr, Loop, Tag, WhenArm, WhenExpr,
    for_loop_pattern_names, is_pattern_as_tag, tag_pattern_binding_names,
};
use internment::Intern;
use std::collections::{HashMap, HashSet};

/// Analyzes control flow to track type narrowing.
pub struct FlowAnalyzer<'a> {
    ty_env: &'a TyEnv,
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

    // NOTE: Constant propagation is implemented via eval_const / extract_pattern_constants.
    // - `val Maybe(3): Some(3)` → val holds Some(3)
    // - `is Some(v)` pattern extraction → v = 3
    // - `four: v + 1` → four = 4 via constant folding
    // TODO: constant propagation through reassignment in loops (i: i + 1 where i = 0)
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

    /// Analyze a spanned expression, recording its SpanId for position-aware context lookup
    /// before delegating to `analyze_expr`.
    fn analyze_spanned_expr(&mut self, expr: &Spanned<Expr>) {
        self.result.expr_spans.insert(expr.1, self.expr_index);
        self.analyze_expr(expr);
    }

    /// Try to evaluate an expression to a compile-time constant value.
    fn eval_const(&self, expr: &Expr) -> Option<ConstValue> {
        match expr {
            Expr::Lit(lit) => match lit {
                ast::Literal::Int(n) => Some(ConstValue::Int(*n as i128)),
                ast::Literal::Float(f) => Some(ConstValue::Float(*f)),
                ast::Literal::String(s) => Some(ConstValue::String(s.clone())),
                ast::Literal::Number(n) => Some(ConstValue::Int(*n as i128)),
            },
            Expr::TagCall(call) => {
                let mut args = Vec::new();
                for arg in &call.args {
                    args.push(self.eval_const(&arg.0)?);
                }
                Some(ConstValue::Tag {
                    name: call.name,
                    args,
                })
            }
            Expr::Binary(bin) => {
                let lhs = self.eval_const(&bin.lhs.0)?;
                let rhs = self.eval_const(&bin.rhs.0)?;
                lhs.eval_binop(&bin.op, &rhs)
            }
            // Lowercase variable reference — look up its known constant value.
            Expr::FnCall(call) if call.args.is_none() && call.path.segments.is_empty() => self
                .current_context()
                .get_constant(&call.path.root)
                .cloned(),
            _ => None,
        }
    }

    /// Extract comparison constraints from a boolean expression.
    ///
    /// For `i < len`, returns `[(i, Compare { op: LessThan, bound: Variable(len) })]`.
    /// Used by both `while` conditions (narrowing inside body, negated after loop)
    /// and `if` boolean conditions (narrowing inside body, negated after early return).
    /// TODO: Compound conditions (&&, ||) are not yet supported. Examples:
    ///
    /// - `while i < len && buf.(i) != target` → both `i < len` AND `buf.(i) != target` inside body
    /// - `if x > 0 && x < 10` → `x > 0` AND `x < 10` inside body
    /// - `if x < 0 || x > 100` → `x < 0` OR `x > 100` inside body (union of constraints)
    ///
    /// Approach: recurse into `&&` by collecting all sub-comparisons (conjunction).
    /// For `||`, we'd need `TypeConstraint::Or` or only apply constraints that appear
    /// on both sides of the disjunction (intersection). Negation of `||` is `&&` via
    /// De Morgan's, so post-loop/return narrowing would naturally compose.
    fn extract_comparisons(&self, expr: &Expr) -> Vec<(Intern<String>, TypeConstraint)> {
        match expr {
            Expr::Binary(bin) => {
                if let Some(op) = CmpOp::from_ast_binop(&bin.op) {
                    // LHS must be a simple in-scope variable
                    let var = match self.extract_var_name(&bin.lhs.0) {
                        Some(v) if self.in_scope.contains(&v) => v,
                        _ => return Vec::new(),
                    };
                    // RHS must be a variable or a constant
                    let bound = if let Some(rhs_var) = self.extract_var_name(&bin.rhs.0) {
                        Bound::Variable(rhs_var)
                    } else if let Some(val) = self.eval_const(&bin.rhs.0) {
                        Bound::Constant(val)
                    } else {
                        return Vec::new();
                    };
                    return vec![(var, TypeConstraint::Compare { op, bound })];
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Extract constant values from pattern variables when the subject's value is known.
    ///
    /// e.g., if `val` holds `Some(3)` and the pattern is `Some(v)`, sets `v = 3`.
    /// Used for `if val is …` and `when val is … then …` pattern arms.
    fn extract_pattern_constants(&mut self, subject_var: Intern<String>, pattern: &Tag) {
        let tag_name = Intern::<String>::from_ref(pattern.name());
        let params = match pattern {
            Tag::Generic(_, params, _) => params,
            _ => return,
        };

        let const_val = match self.current_context().get_constant(&subject_var).cloned() {
            Some(v) => v,
            None => return,
        };

        if let ConstValue::Tag {
            name: val_name,
            args: val_args,
        } = const_val
            && val_name == tag_name
            && val_args.len() == params.len()
        {
            for (i, (param_name, _)) in params.iter().enumerate() {
                self.current_context_mut()
                    .set_constant(*param_name, val_args[i].clone());
            }
        }
    }

    /// Register names from `ast::tag_pattern_binding_names` in `in_scope`; pass the returned
    /// list to `pop_tag_pattern_bindings` after analyzing the pattern arm body.
    fn push_tag_pattern_bindings(&mut self, tag: &Tag) -> Vec<Intern<String>> {
        let names = tag_pattern_binding_names(tag);
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

    fn enter_bind(&mut self, bind: &Bind) {
        // Add parameters to scope
        if let Some(params) = bind.params().as_ref() {
            for (name, _) in params.iter() {
                self.in_scope.insert(*name);
            }
        }
        // Add self if this is a method
        if bind.receiver_type().is_some() {
            self.in_scope.insert(Intern::<String>::from_ref("self"));
        }
    }

    fn analyze_bind(&mut self, bind: &Bind) {
        self.enter_bind(bind);

        match bind.value() {
            BindValue::Body { exprs, ret } => {
                for expr in exprs {
                    self.analyze_spanned_expr(expr);
                }
                if let Some(ret_expr) = &ret.0 {
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
        // Save context *before* incrementing so the context index matches the
        // span index recorded in `analyze_spanned_expr`.
        self.save_context_for_expr();
        self.expr_index += 1;

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
                    let ty = expr.infer_ty(&self.ty_env.infer_env(&self.locals));
                    self.locals.insert(bind.name(), ty);
                }

                // Track constant value for constant propagation
                if let BindValue::Expr(expr) = bind.value()
                    && let Some(const_val) = self.eval_const(&expr.0)
                {
                    self.current_context_mut()
                        .set_constant(bind.name(), const_val);
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
                self.analyze_spanned_expr(&bin.lhs);
                self.analyze_spanned_expr(&bin.rhs);
            }

            // Tuple operations.
            Expr::TupleLit(exprs) => {
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

            // Buffer operations.
            Expr::BufGet { buf, index } => {
                self.analyze_spanned_expr(buf);
                self.analyze_spanned_expr(index);
                self.check_bounds(buf, index, SpanId::INVALID);
            }
            Expr::BufSet { buf, index, value } => {
                self.analyze_spanned_expr(buf);
                self.analyze_spanned_expr(index);
                self.analyze_spanned_expr(value);
                self.check_bounds(buf, index, SpanId::INVALID);
            }

            // Cast and reference operations.
            Expr::Cast { expr, .. } => {
                self.analyze_spanned_expr(expr);
            }
            Expr::TakePtr(inner) => {
                self.analyze_spanned_expr(inner);
            }
            Expr::TakeRef(inner) => {
                self.analyze_spanned_expr(inner);
            }
            Expr::Deref(inner) => {
                self.analyze_spanned_expr(inner);
            }
            Expr::Negate(inner) => {
                self.analyze_spanned_expr(inner);
            }

            // Tag operations.
            Expr::TagCall(_) => {}
            Expr::AnonymousTag(..) => {}
            Expr::SelfRef(_) => {}

            // Only appears as `if`/`when` pattern or bind type payload (parsed separately).
            Expr::IsPattern(_) | Expr::TypeTag(_) => {}

            // Literals and other expressions.
            Expr::Lit(_) => {}
            Expr::FormatString(_) => {}
            Expr::Range(_) => {}
            Expr::Asm(_) => {}
        }
    }

    fn analyze_if_expr(&mut self, if_expr: &IfExpr) {
        match &if_expr.condition {
            IfCondition::Bool(condition) => {
                self.analyze_spanned_expr(condition);

                // Extract comparison constraints from the condition.
                // e.g. `if num < 10` gives us `num < 10` inside the if body.
                let cond_comparisons = self.extract_comparisons(&condition.0);

                // Analyze body in a new scope with comparison narrowing applied
                self.push_context();
                for (var, constraint) in &cond_comparisons {
                    self.current_context_mut().narrow(*var, constraint.clone());
                }
                for expr in &if_expr.body {
                    self.analyze_spanned_expr(expr);
                }

                // Every gin if block always ends with a `return` (syntax requirement),
                // so any if block unconditionally exits the function early.
                // After an early-returning if, the negated condition holds.
                let narrowed_context = self.context_stack.pop().unwrap();
                self.merge_narrowing_from_branch(&narrowed_context);
            }
            IfCondition::Pattern { subject, pattern } => {
                self.analyze_spanned_expr(subject);

                let Some(tag) = is_pattern_as_tag(&pattern.0) else {
                    self.push_context();
                    for expr in &if_expr.body {
                        self.analyze_spanned_expr(expr);
                    }
                    self.pop_context();
                    return;
                };

                // Extract variable name from subject if it's a simple variable reference
                let var_name = self.extract_var_name(subject);

                if let Some(var) = var_name {
                    let variant_name = Intern::<String>::from_ref(tag.name());

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

                        let pat_bindings = self.push_tag_pattern_bindings(tag);
                        // Extract pattern variables from known constant value
                        self.extract_pattern_constants(var, tag);

                        // Every gin if block always ends with a `return` (syntax requirement),
                        // so any if block unconditionally exits the function early.
                        let has_early_return = true;
                        let _ = self.has_return_in_exprs(&if_expr.body); // kept for any body returns

                        for expr in &if_expr.body {
                            self.analyze_spanned_expr(expr);
                        }

                        self.pop_tag_pattern_bindings(&pat_bindings);

                        let narrowed_context = self.context_stack.pop().unwrap();

                        // If the branch has an early return, merge the narrowing back to parent
                        if has_early_return {
                            self.merge_narrowing_from_branch(&narrowed_context);
                        }
                    } else {
                        // Unknown variant, analyze body without narrowing
                        self.push_context();
                        let pat_bindings = self.push_tag_pattern_bindings(tag);
                        for expr in &if_expr.body {
                            self.analyze_spanned_expr(expr);
                        }
                        self.pop_tag_pattern_bindings(&pat_bindings);
                        self.pop_context();
                    }
                } else {
                    // Complex subject, analyze body without narrowing
                    self.push_context();
                    let pat_bindings = self.push_tag_pattern_bindings(tag);
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
        // If there's a subject, analyze it first
        if let Some(subject) = &when_expr.subject {
            self.analyze_spanned_expr(subject.as_ref());

            let var_name = self.extract_var_name(subject);

            // For pattern matching when, analyze each arm
            for arm in &when_expr.arms {
                match arm {
                    WhenArm::Is { pattern, body } => {
                        let Some(tag) = is_pattern_as_tag(&pattern.0) else {
                            self.push_context();
                            self.analyze_spanned_expr(body);
                            self.pop_context();
                            continue;
                        };
                        let variant_name = Intern::<String>::from_ref(tag.name());

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
                            let pat_bindings = self.push_tag_pattern_bindings(tag);
                            if let Some(var) = var_name {
                                self.extract_pattern_constants(var, tag);
                            }
                            self.analyze_spanned_expr(body);
                            self.pop_tag_pattern_bindings(&pat_bindings);
                            self.pop_context();
                        } else {
                            // Unknown variant, analyze without narrowing
                            self.push_context();
                            let pat_bindings = self.push_tag_pattern_bindings(tag);
                            if let Some(var) = var_name {
                                self.extract_pattern_constants(var, tag);
                            }
                            self.analyze_spanned_expr(body);
                            self.pop_tag_pattern_bindings(&pat_bindings);
                            self.pop_context();
                        }
                    }
                    WhenArm::Cond { condition, body } => {
                        self.analyze_spanned_expr(condition);
                        self.push_context();
                        self.analyze_spanned_expr(body);
                        self.pop_context();
                    }
                    WhenArm::Else(body) => {
                        self.push_context();
                        self.analyze_spanned_expr(body);
                        self.pop_context();
                    }
                }
            }
        } else {
            // Boolean when - no narrowing
            for arm in &when_expr.arms {
                match arm {
                    WhenArm::Cond { condition, body } => {
                        self.analyze_spanned_expr(condition);
                        self.analyze_spanned_expr(body);
                    }
                    WhenArm::Else(body) => {
                        self.analyze_spanned_expr(body);
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
                self.analyze_spanned_expr(&while_loop.cond);

                // Extract comparison constraints from the condition.
                // e.g. `while i < len` gives us `i < len` inside the loop body.
                let cond_comparisons = self.extract_comparisons(&while_loop.cond.0);

                // Save entry context for post-loop restoration
                let entry_context = self.current_context().clone();

                // TODO: Constant propagation through reassignment in loops.
                // `i: i + 1` where `i = 0` should track `i = 1` after the first iteration,
                // but currently reset_loop_variables drops the constant before the body is
                // evaluated. A fixpoint approach would iterate until constraints stabilize,
                // widening numeric bounds to avoid infinite ascent (e.g. `i` → 0..∞).
                // For now, comparison constraints like `i < len` are still valid inside the
                // body because the condition is re-checked each iteration.

                // Loop body: narrow based on condition being true.
                // The condition is re-checked each iteration, so the narrowing
                // is valid even though the body may modify variables.
                self.push_context();
                for (var, constraint) in &cond_comparisons {
                    self.current_context_mut().narrow(*var, constraint.clone());
                }
                for expr in &while_loop.exprs {
                    self.analyze_spanned_expr(expr);
                }
                self.pop_context();

                // After loop, reset narrowing for variables modified in loop
                self.reset_loop_variables(&while_loop.exprs, entry_context);

                // After the loop exits, the condition is false.
                // Apply negated comparisons (e.g. `i >= len` after `while i < len`).
                // This is correct because the loop exits precisely when the condition
                // fails — the condition is evaluated at the top of each iteration with
                // the current values of all variables.
                for (var, constraint) in &cond_comparisons {
                    let negated = constraint.negate();
                    self.current_context_mut().narrow(*var, negated);
                }
            }
            Loop::ForIn(for_loop) => {
                // Analyze iterator
                self.analyze_spanned_expr(&for_loop.iter);

                let entry_context = self.current_context().clone();

                // Add loop variable(s) to scope (from pattern)
                if let Some(names) = for_loop_pattern_names(&for_loop.pat.0) {
                    for n in &names {
                        self.in_scope.insert(*n);
                    }
                }

                self.push_context();
                for expr in &for_loop.exprs {
                    self.analyze_spanned_expr(expr);
                }
                self.pop_context();

                // Remove loop variable(s) from scope
                if let Some(names) = for_loop_pattern_names(&for_loop.pat.0) {
                    for n in &names {
                        self.in_scope.remove(n);
                    }
                }

                // Reset narrowing for variables modified in loop
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

    /// Check if a buffer access is out of bounds at compile time.
    ///
    /// Only catches literal indices on arrays with known sizes.
    /// For runtime bounds checking with type narrowing, this is a TODO.
    fn check_bounds(&mut self, buf: &Expr, index: &Expr, span: SpanId) {
        use crate::{TyInfer, TyInferEnv};

        let env = TyInferEnv {
            tag_types: &self.ty_env.tag_types,
            fn_return_types: &self.ty_env.fn_return_types,
            locals: &self.locals,
        };

        let index_val = match index.infer_ty(&env) {
            Ty::Int { value: Some(n), .. } => n,
            Ty::Int { .. } => return,
            _ => return,
        };

        let buf_size = match buf.infer_ty(&env) {
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

    fn extract_var_name(&self, expr: &Expr) -> Option<Intern<String>> {
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
            Expr::SelfRef(_) => Some(Intern::<String>::from_ref("self")),
            _ => None,
        }
    }

    fn has_return_in_exprs(&self, exprs: &[Spanned<Expr>]) -> bool {
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
                let negated = constraint.negate();
                self.current_context_mut().narrow(*var, negated);
                // Clear constant value since the variable is now known to NOT be that variant
                self.current_context_mut().reset_constant(var);
            }
        }
    }

    fn reset_loop_variables(&mut self, exprs: &[Spanned<Expr>], entry_context: FlowContext) {
        // Reset narrowing for variables that are assigned in the loop
        for expr in exprs {
            if let Expr::Bind(bind) = &**expr {
                self.current_context_mut().reset(&bind.name());
            }
        }
        // Restore variables that weren't modified in the loop
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
        result.union_to_variants = self.ty_env.build_union_to_variants();
        result
    }
}
