use diagnostic::{Category, Diagnostic, DiagnosticCode, TypeSymptom};

use crate::analysis::flow::{
    Bound, Capability, ConstValue, FlowAnalysis, FlowContext, ImpossibleCheck, IndexOutOfBounds,
    TypeConstraint, VarState,
};
use crate::analysis::resolve::is_type_surface;
use crate::ty::Ty;
use crate::{
    Bind, BindValue, Expr, FileAst, FnCall, IfCondition, IfExpr, Loop, TypeExpr, Typed, WhenArm,
    WhenExpr, for_loop_pattern_names, pattern_type_binding_names, type_surface_mangle_name,
};
use crate::{TyInfer, TyInferEnv};
use internment::Intern;
use std::collections::{HashMap, HashSet};

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
    /// Track which variables are const (`:=`) bindings (moved = permanent).
    const_vars: HashSet<Intern<String>>,
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
            const_vars: HashSet::new(),
            diagnostics: Vec::new(),
        }
    }

    // NOTE: Constant propagation is implemented via eval_const / extract_pattern_constants.
    // - `val Maybe(3): Some(3)` → val holds Some(3)
    // - `is Some(v)` pattern extraction → v = 3
    // - `four: v + 1` → four = 4 via constant folding
    // TODO: constant propagation through reassignment in loops (i: i + 1 where i = 0)
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

    /// Analyze a spanned expression, recording its SpanId for position-aware context lookup
    /// before delegating to `analyze_expr`.
    fn analyze_spanned_expr(&mut self, expr: &Typed<Expr>) {
        self.result.expr_spans.insert(expr.span_id, self.expr_index);
        self.analyze_expr(expr);
    }

    /// Try to evaluate an expression to a compile-time constant value.
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
                        Expr::Bind(bind) if !bind.is_const => {
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
            Expr::Bind(bind) if bind.is_const || bind.params().is_none() => {
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
            // Look up variable's known constant value
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
                if bin.op.is_comparison() {
                    // LHS must be a simple in-scope variable
                    let var = match self.extract_var_name(&bin.lhs.value) {
                        Some(v) if self.in_scope.contains(&v) => v,
                        _ => return Vec::new(),
                    };
                    // RHS must be a variable or a constant
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

    /// Extract constant values from pattern variables when the subject's value is known.
    ///
    /// e.g., if `val` holds `Some(3)` and the pattern is `Some(v)`, sets `v = 3`.
    /// Used for `if val is …` and `when val is … then …` pattern arms.
    fn extract_pattern_constants(&mut self, subject_var: Intern<String>, pattern: &TypeExpr) {
        let tag_name = Intern::<String>::from_ref(type_surface_mangle_name(pattern));
        let params = match pattern {
            TypeExpr::Generic { params, .. } => params,
            _ => return,
        };

        let const_val = match self.current_context().get_constant(&subject_var).cloned() {
            Some(v) => v,
            None => return,
        };

        if let ConstValue::Tag {
            name: val_name,
            args: val_args,
            ..
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

    /// Register names from `ast::pattern_type_binding_names` in `in_scope`; pass the returned
    /// list to `pop_tag_pattern_bindings` after analyzing the pattern arm body.
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

    fn enter_bind(&mut self, bind: &Bind) {
        if let Some(params) = bind.params().as_ref() {
            for (name, _) in params.iter() {
                self.in_scope.insert(*name);
                self.current_context_mut()
                    .set_var_state(*name, VarState::Alive);
                self.current_context_mut()
                    .set_capability(*name, Capability::Own);
                self.current_context_mut().set_region_owner(*name, *name);
            }
        }
        if bind.is_method() {
            let self_name = Intern::<String>::from_ref("self");
            self.in_scope.insert(self_name);
            self.current_context_mut()
                .set_var_state(self_name, VarState::Alive);
            self.current_context_mut()
                .set_capability(self_name, Capability::Own);
        }
    }

    fn analyze_bind(&mut self, bind: &Bind) {
        self.enter_bind(bind);

        self.current_context_mut()
            .set_var_state(bind.name(), VarState::Alive);
        self.current_context_mut()
            .set_capability(bind.name(), Capability::Own);

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
                if bind.is_const
                    && let Some(const_val) = self.eval_const(&expr.value)
                {
                    self.current_context_mut()
                        .set_constant(bind.name(), const_val);
                }
                self.analyze_spanned_expr(expr);
            }
            BindValue::Extern => {}
        }
    }

    fn analyze_expr(&mut self, expr: &Expr) {
        // Save context before incrementing so the index matches the span index
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

                if bind.is_const {
                    self.const_vars.insert(bind.name());
                }

                if !bind.is_const {
                    self.current_context_mut().reset(&bind.name());
                    self.current_context_mut()
                        .set_var_state(bind.name(), VarState::Alive);
                    self.current_context_mut()
                        .set_capability(bind.name(), Capability::Own);
                    self.reassigned.insert(bind.name());
                }

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

                if let BindValue::Expr(expr) = bind.value()
                    && let Some(const_val) = self.eval_const(&expr.value)
                {
                    self.current_context_mut()
                        .set_constant(bind.name(), const_val);
                }

                self.analyze_bind(bind);
            }

            // Don't persist narrowing across loop iterations
            Expr::Loop(loop_expr) => {
                self.analyze_loop(loop_expr);
            }

            Expr::FnCall(call) => {
                // Check if this is a variable reference (no args, no path segments)
                if call.args.is_none() && call.path.segments.is_empty() {
                    let var_name = call.path.root;
                    if self.in_scope.contains(&var_name) {
                        let ctx = self.current_context();
                        if let Some(VarState::Moved | VarState::MovedButSlotAlive) =
                            ctx.get_var_state(&var_name)
                        {
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
                self.check_bounds(buf, index, crate::span::SpanId::INVALID);
            }
            Expr::BufSet { buf, index, value } => {
                self.analyze_spanned_expr(buf);
                self.analyze_spanned_expr(index);
                self.analyze_spanned_expr(value);
                self.check_bounds(buf, index, crate::span::SpanId::INVALID);
            }

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
            // Owned call argument: mark the referenced variable as Moved
            Expr::OwnArg(inner) => {
                self.analyze_spanned_expr(inner);
                // If the inner expression is a simple variable reference, mark it as moved
                if let Expr::FnCall(call) = &inner.value
                    && call.args.is_none()
                    && call.path.segments.is_empty()
                {
                    let var_name = call.path.root;
                    if self.in_scope.contains(&var_name) {
                        // Const (`:=`) bindings are permanently moved.
                        // Slot (`:`) bindings and parameters can be reassigned.
                        let state = if self.const_vars.contains(&var_name) {
                            VarState::Moved
                        } else {
                            VarState::MovedButSlotAlive
                        };
                        self.current_context_mut().set_var_state(var_name, state);
                        self.current_context_mut().consume_region(&var_name);
                    }
                }
            }
            // Mutable call argument: check that the variable has at least Write capability
            Expr::MutArg(inner) => {
                self.analyze_spanned_expr(inner);
                // When passing as `mut`, downgrade the capability temporarily
                if let Expr::FnCall(call) = &inner.value
                    && call.args.is_none()
                    && call.path.segments.is_empty()
                {
                    let var_name = call.path.root;
                    if self.in_scope.contains(&var_name) {
                        // Check that the variable has at least Write capability
                        if let Some(cap) = self.current_context().get_capability(&var_name)
                            && cap < Capability::Write
                        {
                            self.diagnostics.push(Diagnostic {
                                    code: DiagnosticCode::Type(
                                        TypeSymptom::CannotPassReadonlyAsMut {
                                            name: var_name.as_str().to_string(),
                                        },
                                    ),
                                    message: format!(
                                        "cannot pass `{}` as `mut` because it is read-only",
                                        var_name.as_str()
                                    ),
                                    help_on_span: None,
                                    help: Some(
                                        "declare the parameter with `mut` or bind with `:` instead of `:="
                                            .into(),
                                    ),
                                    span_id: crate::span::SpanId::INVALID,
                                    category: Category::Flaw,
                                    related: Vec::new(),
                                });
                        }
                        // Downgrade to Write while passed to callee
                        self.current_context_mut()
                            .set_capability(var_name, Capability::Write);
                    }
                }
            }

            Expr::TagCall(_) => {}
            Expr::AnonymousTag(..) => {}
            Expr::SelfRef(_) => {}

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

                // Extract comparison constraints from the condition.
                // e.g. `if num < 10` gives us `num < 10` inside the if body.
                let cond_comparisons = self.extract_comparisons(&condition.value);

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
                        self.extract_pattern_constants(var, &pattern.value);

                        // Every gin if block always ends with a `return` (syntax requirement),
                        // so any if block unconditionally exits the function early.
                        let has_early_return = true;
                        let _ = self.has_return_in_exprs(&if_expr.body); // kept for any body returns

                        for expr in &if_expr.body {
                            self.analyze_spanned_expr(expr);
                        }

                        self.pop_tag_pattern_bindings(&pat_bindings);

                        let narrowed_context = self.context_stack.pop().unwrap();

                        if has_early_return {
                            self.merge_narrowing_from_branch(&narrowed_context);
                        }
                    } else {
                        // Unknown variant, analyze body without narrowing
                        self.push_context();
                        let pat_bindings = self.push_pattern_bindings(&pattern.value);
                        for expr in &if_expr.body {
                            self.analyze_spanned_expr(expr);
                        }
                        self.pop_tag_pattern_bindings(&pat_bindings);
                        self.pop_context();
                    }
                } else {
                    // Complex subject, analyze body without narrowing
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
                            if let Some(var) = var_name {
                                self.extract_pattern_constants(var, &pattern.value);
                            }
                            self.analyze_spanned_expr(body);
                            self.pop_tag_pattern_bindings(&pat_bindings);
                            self.pop_context();
                        } else {
                            self.push_context();
                            let pat_bindings = self.push_pattern_bindings(&pattern.value);
                            if let Some(var) = var_name {
                                self.extract_pattern_constants(var, &pattern.value);
                            }
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

                // Extract comparison constraints from the condition
                let cond_comparisons = self.extract_comparisons(&while_loop.cond.value);

                let entry_context = self.current_context().clone();

                // Narrow based on condition (re-checked each iteration)
                self.push_context();
                for (var, constraint) in &cond_comparisons {
                    self.current_context_mut().narrow(*var, constraint.clone());
                }
                for expr in &while_loop.exprs {
                    self.analyze_spanned_expr(expr);
                }
                self.pop_context();

                self.reset_loop_variables(&while_loop.exprs, entry_context);

                // After the loop exits, the negated condition holds (e.g. `i >= len` after `while i < len`)
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

    /// Check if a buffer access is out of bounds at compile time.
    ///
    /// Only catches literal indices on arrays with known sizes.
    /// For runtime bounds checking with type narrowing, this is a TODO.
    fn check_bounds(&mut self, buf: &Expr, index: &Expr, span: crate::span::SpanId) {
        let env = TyInferEnv {
            tag_types: self.tag_types,
            fn_return_types: self.fn_return_types,
            locals: &self.locals,
            tag_params: None,
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

    fn has_return_in_exprs(&self, exprs: &[Typed<Expr>]) -> bool {
        exprs.iter().any(|e| self.has_return(e))
    }

    fn has_return(&self, expr: &Typed<Expr>) -> bool {
        match &**expr {
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
                        || ret.value.as_ref().is_some_and(|e| self.has_return(e))
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

    fn reset_loop_variables(&mut self, exprs: &[Typed<Expr>], entry_context: FlowContext) {
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
