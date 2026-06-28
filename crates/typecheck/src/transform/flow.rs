//! Stage 3: Flow — Compute flow contexts, attach flow flaws.
//!
//! This stage walks the typed expression arena depth-first, computes
//! `FlowContext` at each program point, and attaches flow-related flaws
//! (ownership, bounds, narrowing, lin-value checks, etc.).

use std::cell::RefCell;
use std::collections::HashMap;

use crate::analysis::infer_convention::ConventionInference;
use crate::analysis::{FlowContext, VarState};
use crate::reflect::reflect_ty_to_const_value;
use crate::solver::{ProveResult, predicate_expr_to_predicate};
use ast::const_expr::ConstExpr;
use ast::expr::Literal;
use ast::{ConstValue, HashFloat};
use diagnostic::Diagnostic;

use crate::ty::Ty;
use crate::typed::{
    DefId, ExprId, TypedExprKind, TypedFileAst, TypedLoopKind, TypedWhenArm, VariantId,
};
use internment::Intern;

/// Build a map from local bind variable names to their resolved types
/// by scanning the typed expression arena for `TypedExprKind::Bind` entries.
fn collect_local_var_types(typed: &TypedFileAst) -> HashMap<Intern<String>, Ty> {
    let mut var_types = HashMap::new();
    for bind in typed.defs.values() {
        if bind.unassigned_decl {
            var_types.insert(bind.name, bind.return_type.clone());
        }
    }
    for (idx, kind) in typed.exprs.kind.iter().enumerate() {
        if let TypedExprKind::Bind { name, .. } = kind {
            let ty = &typed.exprs.ty[idx];
            var_types.insert(*name, ty.clone());
        }
    }
    var_types
}

thread_local! {
    static REF_TRACKER: RefCell<Option<RefTracker>> = const { RefCell::new(None) };
}

/// Clear the ref tracker after stage_flow completes.
fn clear_ref_tracker() {
    REF_TRACKER.with(|rt| {
        *rt.borrow_mut() = None;
    });
}

/// Tracks ref-to-source dependencies for child-group invalidation.
pub struct RefTracker {}

impl RefTracker {
    fn new() -> Self {
        Self {}
    }
}

/// Walks the expression arena, computes flow contexts at each program point,
/// and attaches flow-related flaws to expressions.
pub fn stage_flow(typed: &mut TypedFileAst, conventions: &HashMap<DefId, ConventionInference>) {
    // Collect root expression IDs from all bind bodies.
    let mut def_ids: Vec<_> = typed.defs.keys().copied().collect();
    def_ids.sort_by_key(|def_id| {
        let bind = typed.defs.get(def_id);
        let is_value = bind.is_some_and(|bind| bind.params.is_empty());
        (!is_value, def_id.0.as_str().to_string())
    });
    let bind_bodies: Vec<(DefId, Vec<ExprId>)> = def_ids
        .into_iter()
        .filter_map(|def_id| {
            let bind = typed.defs.get(&def_id)?;
            let ids = match &bind.body {
                crate::typed::BindBody::Expr(eid) => vec![*eid],
                crate::typed::BindBody::Body { exprs, ret } => {
                    let mut v: Vec<ExprId> = exprs.clone();
                    v.extend(ret);
                    v
                }
                crate::typed::BindBody::Extern => return None,
            };
            Some((def_id, ids))
        })
        .collect();

    let local_var_types = collect_local_var_types(typed);

    let mut ref_tracker = RefTracker::new();

    // Track constants detected at compile time.
    let mut const_bindings: HashMap<Intern<String>, bool> = HashMap::new();

    // Process each bind body.
    for (_def_id, root_ids) in &bind_bodies {
        let mut flow_ctx = FlowContext::new();
        let mut local_const_values: HashMap<Intern<String>, ConstValue> = HashMap::new();
        let mut ctx = WalkExprCtx {
            typed,
            flow_ctx: &mut flow_ctx,
            local_var_types: &local_var_types,
            ref_tracker: &mut ref_tracker,
            const_bindings: &mut const_bindings,
            conventions,
            local_const_values: &mut local_const_values,
        };
        for root_id in root_ids {
            ctx.walk(*root_id);
        }
        emit_unassigned_bindings(ctx.typed, ctx.flow_ctx);
    }

    // Process top-level expressions.
    let mut flow_ctx = FlowContext::new();
    let mut local_const_values: HashMap<Intern<String>, ConstValue> = HashMap::new();
    let root_ids = typed.root_exprs.clone();
    let mut ctx = WalkExprCtx {
        typed,
        flow_ctx: &mut flow_ctx,
        local_var_types: &local_var_types,
        ref_tracker: &mut ref_tracker,
        const_bindings: &mut const_bindings,
        conventions,
        local_const_values: &mut local_const_values,
    };
    for root_id in &root_ids {
        ctx.walk(*root_id);
    }

    clear_ref_tracker();
}

fn emit_unassigned_bindings(typed: &mut TypedFileAst, flow_ctx: &FlowContext) {
    let unassigned: Vec<(usize, Intern<String>)> = typed
        .exprs
        .kind
        .iter()
        .enumerate()
        .filter_map(|(idx, kind)| match kind {
            TypedExprKind::Bind {
                name,
                unassigned: true,
                ..
            } if matches!(flow_ctx.get_var_state(name), Some(VarState::Declared)) => {
                Some((idx, *name))
            }
            _ => None,
        })
        .collect();
    for (idx, name) in unassigned {
        if !typed.exprs.flaws[idx].iter().any(|f| {
            f.code.slug() == "type-unassigned-binding" && f.arg("name") == Some(name.as_str())
        }) {
            typed.exprs.flaws[idx].push(
                Diagnostic::new(
                    "type-unassigned-binding",
                    format!("binding `{}` is declared but never assigned", name.as_str()),
                )
                .with_arg("name", name.as_str().to_string()),
            );
        }
    }
}

fn mark_consumed_if_local_ref(typed: &TypedFileAst, expr_id: ExprId, flow_ctx: &mut FlowContext) {
    let idx = expr_id.as_usize();
    if idx >= typed.exprs.kind.len() {
        return;
    }
    match &typed.exprs.kind[idx] {
        TypedExprKind::Bind { name, .. } => {
            flow_ctx.set_var_state(*name, VarState::Consumed);
        }
        TypedExprKind::FnCall { target, args } if args.as_ref().is_none_or(|a| a.is_empty()) => {
            flow_ctx.set_var_state(target.0, VarState::Consumed);
        }
        _ => {}
    }
}

/// Shared context threaded through flow analysis expression walking.
#[allow(dead_code)]
struct WalkExprCtx<'a> {
    typed: &'a mut TypedFileAst,
    flow_ctx: &'a mut FlowContext,
    local_var_types: &'a HashMap<Intern<String>, Ty>,
    ref_tracker: &'a mut RefTracker,
    const_bindings: &'a mut HashMap<Intern<String>, bool>,
    conventions: &'a HashMap<DefId, ConventionInference>,
    local_const_values: &'a mut HashMap<Intern<String>, ConstValue>,
}

impl<'a> WalkExprCtx<'a> {
    fn walk(&mut self, expr_id: ExprId) {
        let idx = expr_id.as_usize();
        if idx >= self.typed.exprs.kind.len() {
            return;
        }

        // Save context for this expression
        self.typed.exprs.const_value[idx] = self.typed.exprs.const_value[idx].clone();

        match self.typed.exprs.kind[idx].clone() {
            TypedExprKind::Lit(_) => {}
            TypedExprKind::Binary { lhs, rhs, .. } => {
                self.walk(lhs);
                self.walk(rhs);
            }
            TypedExprKind::FnCall { target, args } => {
                if args.as_ref().is_none_or(|a| a.is_empty())
                    && let Some(cv) = self.local_const_values.get(&target.0).cloned()
                {
                    self.typed.exprs.const_value[idx] = Some(cv);
                }
                if args.as_ref().is_none_or(|a| a.is_empty()) {
                    match self.flow_ctx.get_var_state(&target.0) {
                        Some(VarState::Consumed) => {
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-use-of-moved-value",
                                    format!("use of moved value `{}`", target.0.as_str()),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                        }
                        Some(VarState::Declared) => {
                            self.typed.exprs.flaws[idx].push(
                                Diagnostic::new(
                                    "type-use-before-assign",
                                    format!("use before assign `{}`", target.0.as_str()),
                                )
                                .with_arg("name", target.0.as_str().to_string()),
                            );
                        }
                        _ => {}
                    }
                }
                if let Some(arg_ids) = args {
                    for arg_id in &arg_ids {
                        self.walk(*arg_id);
                    }
                }
            }
            TypedExprKind::TagCall {
                variant_id,
                args,
                field_names,
                ..
            } => {
                if let Some(arg_ids) = &args {
                    for arg_id in arg_ids.iter().copied() {
                        self.walk(arg_id);
                    }
                }
                // Check refinements for this TagCall
                self.check_tag_refinements(&variant_id, &args, &field_names);
            }
            TypedExprKind::Bind {
                name,
                stmts,
                body,
                unassigned,
            } => {
                for stmt in stmts {
                    if stmt.as_usize() == idx {
                        continue;
                    }
                    self.walk(stmt);
                }
                if !unassigned && body.as_usize() != idx {
                    self.walk(body);
                }
                // Track bind as a constant if its body has a const_value.
                let bind_const = self.typed.exprs.const_value[body.as_usize()]
                    .clone()
                    .or_else(|| self.typed.exprs.const_value[idx].clone());
                self.typed.exprs.const_value[idx] = bind_const.clone();
                if let Some(cv) = bind_const {
                    self.const_bindings.insert(name, true);
                    self.local_const_values.insert(name, cv);
                } else {
                    self.local_const_values.remove(&name);
                }
                if unassigned {
                    self.flow_ctx.set_var_state(name, VarState::Declared);
                } else {
                    self.flow_ctx.set_var_state(name, VarState::Alive);
                }
            }
            TypedExprKind::Reassign { name, value } => {
                self.walk(value);
                self.flow_ctx.set_var_state(name, VarState::Alive);
            }
            TypedExprKind::Eat(inner) => {
                self.walk(inner);
                mark_consumed_if_local_ref(self.typed, inner, self.flow_ctx);
            }
            TypedExprKind::TakePtr(inner) | TypedExprKind::Ref(inner) => {
                self.walk(inner);
            }
            TypedExprKind::When(when_expr) => {
                if let Some(subject_id) = when_expr.subject {
                    self.walk(subject_id);
                }
                let subject_value = when_expr.subject.and_then(|subject_id| {
                    self.typed.exprs.const_value[subject_id.as_usize()].clone()
                });
                let mut selected_body = None;
                if let Some(subject_value) = &subject_value {
                    for arm in &when_expr.arms {
                        match arm {
                            TypedWhenArm::Is { pattern, body, .. } => {
                                let matches =
                                    crate::analysis::pattern::pattern_matches_with_tag_types(
                                        &pattern.value,
                                        subject_value,
                                        &self.typed.tag_types,
                                    ) || super::lower_exprs::pattern_value_const(
                                        &pattern.value,
                                        self.typed,
                                    )
                                    .as_ref()
                                    .is_some_and(|cv| cv == subject_value);
                                if matches {
                                    selected_body = Some(*body);
                                    break;
                                }
                                self.typed.exprs.flaws[idx].push(Diagnostic::new(
                                    "type-unreachable-when-arm",
                                    "when arm is unreachable",
                                ));
                            }
                            TypedWhenArm::Else(body, _) => {
                                selected_body = Some(*body);
                                break;
                            }
                            TypedWhenArm::Cond { .. } => {}
                        }
                    }
                }
                if let Some(body) = selected_body {
                    self.walk(body);
                    self.typed.exprs.const_value[idx] =
                        self.typed.exprs.const_value[body.as_usize()].clone();
                } else {
                    for arm in &when_expr.arms {
                        match arm {
                            TypedWhenArm::Cond {
                                condition, body, ..
                            } => {
                                self.walk(*condition);
                                self.walk(*body);
                            }
                            TypedWhenArm::Is { body, .. } => {
                                self.walk(*body);
                            }
                            TypedWhenArm::Else(body, _) => {
                                self.walk(*body);
                            }
                        }
                    }
                }
            }
            TypedExprKind::If(if_expr) => {
                let comparison_facts = subject_comparison_facts(
                    &self.typed.exprs.kind,
                    if_expr.subject,
                    self.local_const_values,
                );
                let saved_lt_len = self.flow_ctx.constraint_env.known_lt.len();
                let saved_le_len = self.flow_ctx.constraint_env.known_le.len();
                for (lhs, rhs, is_lt) in &comparison_facts {
                    if *is_lt {
                        self.flow_ctx
                            .constraint_env
                            .known_lt
                            .push((lhs.clone(), rhs.clone()));
                    } else {
                        self.flow_ctx
                            .constraint_env
                            .known_le
                            .push((lhs.clone(), rhs.clone()));
                    }
                }
                self.walk(if_expr.subject);
                for body_id in if_expr.stmts {
                    self.walk(body_id);
                }
                if let Some(ret_id) = if_expr.ret {
                    self.walk(ret_id);
                }
                // Restore constraint_env so facts don't leak past the if-block
                self.flow_ctx.constraint_env.known_lt.truncate(saved_lt_len);
                self.flow_ctx.constraint_env.known_le.truncate(saved_le_len);
            }
            TypedExprKind::Loop(typed_loop) => {
                match typed_loop.kind {
                    TypedLoopKind::While { condition } => {
                        self.walk(condition);
                    }
                    TypedLoopKind::ForIn {
                        variable: _,
                        iterable,
                    } => {
                        self.walk(iterable);
                    }
                }
                for body_id in typed_loop.stmts {
                    self.walk(body_id);
                }
            }
            TypedExprKind::TupleGet { base, .. } => {
                self.walk(base);
            }
            TypedExprKind::Destructure { value, .. } => {
                self.walk(value);
            }
            TypedExprKind::RecordSet { base, value, .. }
            | TypedExprKind::TupleSet { base, value, .. } => {
                self.walk(base);
                self.walk(value);
            }
            TypedExprKind::Range { start, end } => {
                self.walk(start);
                self.walk(end);
            }
            TypedExprKind::TupleAlloc { init, .. } => {
                self.walk(init);
            }
            TypedExprKind::TupleLit(items) | TypedExprKind::List(items) => {
                for item_id in items {
                    self.walk(item_id);
                }
            }
            TypedExprKind::BufGet { buf, index } | TypedExprKind::BufSet { buf, index, .. } => {
                self.walk(buf);
                self.walk(index);
            }
            TypedExprKind::Cast { expr: inner, .. } => {
                self.walk(inner);
            }
            TypedExprKind::ConsumeArg(inner) => {
                self.walk(inner);
                mark_consumed_if_local_ref(self.typed, inner, self.flow_ctx);
            }
            TypedExprKind::Deref(inner) | TypedExprKind::Negate(inner) => {
                self.walk(inner);
            }
            TypedExprKind::FormatString(fs) => {
                for part in &fs.parts {
                    if let ast::FormatPart::Expr(_typed_expr, _) = part {
                        // _typed_expr.value is ast::Expr — not an ExprId, so we skip
                        // walking children here. The FormatString's inner expressions
                        // are untyped and won't have flow context attached.
                    }
                }
            }
            TypedExprKind::Asm(_asm_expr) => {
                // TODO: walk asm operand expressions once they carry ExprId
            }
            TypedExprKind::SelfRef { .. } => {}
        }
    }

    fn check_tag_refinements(
        &mut self,
        variant_id: &VariantId,
        args: &Option<Vec<ExprId>>,
        field_names: &[Intern<String>],
    ) {
        let Some(tag) = self.typed.tags.get(&variant_id.union) else {
            return;
        };
        let refinements = &tag.record_field_refinements;
        if refinements.is_empty() {
            return;
        }
        let Some(arg_ids) = args.as_ref() else {
            return;
        };
        let ordered_fields: Vec<Intern<String>> = match &tag.resolved_ty {
            crate::ty::Ty::Record { fields, .. } => fields.iter().map(|(n, _)| *n).collect(),
            _ => return,
        };
        // Build a map of all field values so refinements can reference sibling fields.
        let all_field_values: HashMap<Intern<String>, ConstExpr> = {
            let mut m = HashMap::new();
            for (j, arg_id) in arg_ids.iter().enumerate() {
                let fname = field_names
                    .get(j)
                    .filter(|n| !n.as_str().is_empty())
                    .copied()
                    .or_else(|| ordered_fields.get(j).copied());
                if let Some(fname) = fname
                    && let Some(cv) = self.typed.exprs.const_value[arg_id.as_usize()].clone()
                {
                    m.insert(fname, ConstExpr::Value(cv));
                }
            }
            m
        };
        let span_table = &self.typed.span_table;
        for (i, arg_id) in arg_ids.iter().enumerate() {
            let field_name = field_names
                .get(i)
                .filter(|n| !n.as_str().is_empty())
                .copied()
                .or_else(|| ordered_fields.get(i).copied());
            let Some(field_name) = field_name else {
                continue;
            };
            let Some(refinement) = refinements.get(&field_name) else {
                continue;
            };
            let arg_cv = self.typed.exprs.const_value[arg_id.as_usize()].clone();
            let Some(cv) = arg_cv else {
                continue;
            };
            let field_value = ConstExpr::Value(cv);
            // Include sibling field values so the refinement can reference them.
            // e.g. for `Index(n) with value and < n`, the `value` refinement
            // references the `n` field — we need both in var_values.
            let mut var_values: HashMap<Intern<String>, ConstExpr> = self
                .local_const_values
                .iter()
                .map(|(k, v)| (*k, ConstExpr::Value(v.clone())))
                .collect();
            var_values.extend(all_field_values.iter().map(|(k, v)| (*k, v.clone())));
            let predicate = predicate_expr_to_predicate(refinement, field_value, &var_values);
            match self.flow_ctx.constraint_env.prove(&predicate, &var_values) {
                ProveResult::Proven => {}
                ProveResult::Disproven => {
                    let span = self.typed.exprs.span[arg_id.as_usize()];
                    self.typed.exprs.flaws[arg_id.as_usize()].push(
                        Diagnostic::new(
                            "dep-refinement-failed",
                            format!(
                                "refinement `{:?}` is false for field `{}`",
                                refinement,
                                field_name.as_str()
                            ),
                        )
                        .at_span_id(span, span_table),
                    );
                }
                ProveResult::Unknown => {
                    let span = self.typed.exprs.span[arg_id.as_usize()];
                    self.typed.exprs.flaws[arg_id.as_usize()].push(
                        Diagnostic::new(
                            "dep-refinement-unproven",
                            format!(
                                "cannot prove refinement `{:?}` for field `{}`",
                                refinement,
                                field_name.as_str()
                            ),
                        )
                        .at_span_id(span, span_table),
                    );
                }
            }
        }
    }
}

pub(crate) fn children_of_kind(kind: &TypedExprKind) -> Vec<ExprId> {
    let mut children = Vec::new();
    match kind {
        TypedExprKind::Lit(_) => {}
        TypedExprKind::Binary { lhs, rhs, .. } => {
            children.push(*lhs);
            children.push(*rhs);
        }
        TypedExprKind::FnCall { args, .. } => {
            if let Some(arg_ids) = args {
                children.extend(arg_ids.iter().copied());
            }
        }
        TypedExprKind::TagCall { args, .. } => {
            if let Some(arg_ids) = args {
                children.extend(arg_ids.iter().copied());
            }
        }
        TypedExprKind::When(when_expr) => {
            if let Some(subject_id) = when_expr.subject {
                children.push(subject_id);
            }
            for arm in &when_expr.arms {
                match arm {
                    TypedWhenArm::Cond {
                        condition, body, ..
                    } => {
                        children.push(*condition);
                        children.push(*body);
                    }
                    TypedWhenArm::Is { body, .. } => {
                        children.push(*body);
                    }
                    TypedWhenArm::Else(body, _) => {
                        children.push(*body);
                    }
                }
            }
        }
        TypedExprKind::If(if_expr) => {
            children.push(if_expr.subject);
            children.extend(if_expr.stmts.iter().copied());
            if let Some(ret_id) = if_expr.ret {
                children.push(ret_id);
            }
        }
        TypedExprKind::Loop(typed_loop) => {
            match typed_loop.kind {
                TypedLoopKind::While { condition } => children.push(condition),
                TypedLoopKind::ForIn {
                    variable: _,
                    iterable,
                } => {
                    children.push(iterable);
                }
            }
            children.extend(typed_loop.stmts.iter().copied());
        }
        TypedExprKind::TakePtr(inner)
        | TypedExprKind::Ref(inner)
        | TypedExprKind::Eat(inner)
        | TypedExprKind::ConsumeArg(inner)
        | TypedExprKind::Deref(inner)
        | TypedExprKind::Negate(inner) => {
            children.push(*inner);
        }
        TypedExprKind::TupleGet { base, .. } => {
            children.push(*base);
        }
        TypedExprKind::Destructure { value, .. } => {
            children.push(*value);
        }
        TypedExprKind::RecordSet { base, value, .. }
        | TypedExprKind::TupleSet { base, value, .. } => {
            children.push(*base);
            children.push(*value);
        }
        TypedExprKind::Range { start, end } => {
            children.push(*start);
            children.push(*end);
        }
        TypedExprKind::TupleAlloc { init, .. } => {
            children.push(*init);
        }
        TypedExprKind::TupleLit(items) | TypedExprKind::List(items) => {
            children.extend(items.iter().copied());
        }
        TypedExprKind::BufGet { buf, index } => {
            children.push(*buf);
            children.push(*index);
        }
        TypedExprKind::BufSet { buf, index, value } => {
            children.push(*buf);
            children.push(*index);
            children.push(*value);
        }
        TypedExprKind::Cast { expr: inner, .. } => {
            children.push(*inner);
        }
        TypedExprKind::FormatString(fs) => {
            for part in &fs.parts {
                if let ast::FormatPart::Expr(_, _) = part {
                    // Children not accessible directly from untyped FormatPart
                }
            }
        }
        TypedExprKind::Asm(_asm_expr) => {
            // TODO: extract children from asm operand expressions
        }
        TypedExprKind::Bind {
            stmts,
            body,
            unassigned,
            ..
        } => {
            children.extend(stmts.iter().copied());
            if !unassigned {
                children.push(*body);
            }
        }
        TypedExprKind::Reassign { value, .. } => {
            children.push(*value);
        }
        TypedExprKind::SelfRef { .. } => {}
    }
    children
}

pub(crate) fn eval_const_from_expr(typed: &TypedFileAst, idx: usize) -> Option<ConstValue> {
    if idx >= typed.exprs.kind.len() {
        return None;
    }
    match &typed.exprs.kind[idx] {
        TypedExprKind::Lit(lit) => match lit {
            Literal::Int(n) => Some(ConstValue::Int(*n as i128)),
            Literal::Number(n) => Some(ConstValue::Int(*n as i128)),
            Literal::Float(HashFloat(f)) => Some(ConstValue::Float(HashFloat(*f))),
            Literal::String(s) => Some(ConstValue::String(s.clone())),
        },
        TypedExprKind::TagCall { args, .. } if args.as_ref().is_some_and(|a| a.is_empty()) => {
            // Bare tag call with no args - could be a type constant
            let ty = &typed.exprs.ty[idx];
            Some(reflect_ty_to_const_value(ty))
        }
        _ => typed.exprs.const_value[idx].clone(),
    }
}

/// Extract comparison facts from an `if` subject expression.
/// Returns `(lhs, rhs, is_lt)` tuples where `is_lt` means `lhs < rhs`
/// and `!is_lt` means `lhs <= rhs`.
///
/// Comparison operators desugar to function calls:
/// - `a < b`  → `lt(a, b)`  → `known_lt.push((a, b))`
/// - `a <= b` → `le(a, b)`  → `known_le.push((a, b))`
/// - `a > b`  → `gt(a, b)`  → `known_lt.push((b, a))`
/// - `a >= b` → `ge(a, b)`  → `known_le.push((b, a))`
fn subject_comparison_facts(
    kinds: &[TypedExprKind],
    subject: ExprId,
    lcv: &HashMap<Intern<String>, ConstValue>,
) -> Vec<(ConstExpr, ConstExpr, bool)> {
    let idx = subject.as_usize();
    let Some(TypedExprKind::FnCall { target, args }) = kinds.get(idx) else {
        return Vec::new();
    };
    let Some(arg_ids) = args.as_ref() else {
        return Vec::new();
    };
    if arg_ids.len() != 2 {
        return Vec::new();
    }
    let op_name = target.0.as_str();
    let is_swapped = matches!(op_name, "gt" | "ge");
    let is_lt = matches!(op_name, "lt" | "gt");
    let lhs_idx = if is_swapped { 1 } else { 0 };
    let rhs_idx = if is_swapped { 0 } else { 1 };
    let to_expr = |eid: ExprId| -> Option<ConstExpr> {
        let eidx = eid.as_usize();
        match kinds.get(eidx)? {
            TypedExprKind::Lit(lit) => match lit {
                Literal::Int(n) => Some(ConstExpr::Value(ConstValue::Int(*n as i128))),
                Literal::Number(n) => Some(ConstExpr::Value(ConstValue::Int(*n as i128))),
                _ => None,
            },
            TypedExprKind::FnCall { target, args } if args.is_none() => {
                let var = target.0;
                if let Some(cv) = lcv.get(&var) {
                    Some(ConstExpr::Value(cv.clone()))
                } else {
                    Some(ConstExpr::Var(var))
                }
            }
            _ => None,
        }
    };
    let Some(lhs) = to_expr(arg_ids[lhs_idx]) else {
        return Vec::new();
    };
    let Some(rhs) = to_expr(arg_ids[rhs_idx]) else {
        return Vec::new();
    };
    vec![(lhs, rhs, is_lt)]
}
