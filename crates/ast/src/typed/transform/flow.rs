//! Stage 3: Flow — Compute flow contexts, attach flow flaws.
//!
//! This stage walks the typed expression arena depth-first, computes
//! `FlowContext` at each program point, and attaches flow-related flaws
//! (ownership, bounds, narrowing, lin-value checks, etc.).

use std::collections::HashMap;

use crate::analysis::infer_convention::{ConventionInference, InferredConvention};
use crate::analysis::{FlowContext, VarState};
use crate::expr::Literal;
use crate::ty::Ty;
use crate::typed::{
    DefId, ExprId, TypedExprKind, TypedFileAst, TypedIfCondition, TypedLoopKind, TypedWhenArm,
};
use diagnostic::TypeSymptom;
use internment::Intern;

/// Determine whether a type is implicitly copyable.
///
/// Delegates to the marker registry's structural `Copy` inference.
/// Primitives (int, float, bool, unit) are always Copy.
fn is_copyable(ty: &Ty, registry: &crate::marker::MarkerRegistry) -> bool {
    crate::analysis::is_copyable(ty, registry)
}

/// Build a map from local bind variable names to their resolved types
/// by scanning the typed expression arena for `TypedExprKind::Bind` entries.
fn collect_local_var_types(typed: &TypedFileAst) -> HashMap<Intern<String>, Ty> {
    let mut var_types = HashMap::new();
    for (idx, kind) in typed.exprs.kind.iter().enumerate() {
        if let TypedExprKind::Bind { name, .. } = kind {
            let ty = &typed.exprs.ty[idx];
            var_types.insert(*name, ty.clone());
        }
    }
    var_types
}

/// Walks the expression arena, computes flow contexts at each program point,
/// and attaches flow-related flaws to expressions.
pub fn stage_flow(typed: &mut TypedFileAst, conventions: &HashMap<DefId, ConventionInference>) {
    // Collect root expression IDs from all bind bodies.
    let bind_bodies: Vec<(crate::typed::DefId, Vec<ExprId>)> = typed
        .defs
        .keys()
        .copied()
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

    // Walk each bind body with a fresh flow context.
    for (def_id, root_ids) in &bind_bodies {
        let mut flow = FlowContext::new();

        // Set `~` params as Alive before walking the body.
        // Bare params are auto-threaded and handled by the convention system.
        // Only `~` params that stay Alive at scope exit produce a leak.
        let consume_params: std::collections::HashSet<Intern<String>> = conventions
            .get(def_id)
            .map(|conv| {
                conv.conventions
                    .iter()
                    .filter(|(_, c)| **c == InferredConvention::Consumed)
                    .map(|(name, _)| *name)
                    .collect()
            })
            .unwrap_or_default();
        for param_name in &consume_params {
            flow.set_var_state(*param_name, VarState::Alive);
        }

        for root_id in root_ids {
            walk_expr_flow(typed, *root_id, &mut flow, conventions, Some(*def_id));
        }

        // A consumed (`~`) parameter that appears as the return expression
        // (the last expression in the body) cannot be returned — `~` params
        // are dropped at scope exit, so returning them is invalid.
        if let Some(last_id) = root_ids.last() {
            let last_idx = last_id.as_usize();
            if last_idx < typed.exprs.kind.len()
                && let Some(name) = resolve_expr_name_from_kind(&typed.exprs.kind[last_idx])
                && consume_params.contains(&name)
                && flow.get_var_state(&name) == Some(VarState::Alive)
            {
                add_flaw(
                    typed,
                    *last_id,
                    TypeSymptom::ReturnConsumedParam {
                        name: name.as_str().to_string(),
                    },
                );
                // Mark consumed so the auto-drop check doesn't also
                // produce a redundant note for this param.
                flow.set_var_state(name, VarState::Consumed);
            }
        }

        // Check for unconsumed values at scope exit:
        // - local binds of non-Copy types that were never consumed
        // Bare params are auto-threaded through the return — not checked.
        // `~` params are auto-dropped at scope exit, so they are not checked.
        let local_var_types = collect_local_var_types(typed);
        check_lin_values(typed, &flow, &consume_params, &local_var_types);
    }

    // Walk top-level expressions.
    let mut flow = FlowContext::new();
    for root_id in typed.root_exprs.clone() {
        walk_expr_flow(typed, root_id, &mut flow, conventions, None);
    }
}

fn walk_expr_flow(
    typed: &mut TypedFileAst,
    expr_id: ExprId,
    flow: &mut FlowContext,
    conventions: &HashMap<DefId, ConventionInference>,
    current_def_id: Option<DefId>,
) {
    let idx = expr_id.as_usize();
    if idx >= typed.exprs.kind.len() {
        return;
    }

    // Record the flow context at this program point.
    typed.exprs.flow[idx] = Some(flow.clone());

    let kind = typed.exprs.kind[idx].clone();

    // Check for use-after-move: if this expression references a moved variable.
    if let Some(name) = resolve_expr_name_from_kind(&kind)
        && flow.get_var_state(&name) == Some(VarState::Consumed)
    {
        add_flaw(
            typed,
            expr_id,
            TypeSymptom::UseOfMovedValue {
                name: name.as_str().to_string(),
            },
        );
    }

    match kind {
        TypedExprKind::Lit(_) => {}

        TypedExprKind::Binary { lhs, rhs, .. } => {
            walk_expr_flow(typed, lhs, flow, conventions, current_def_id);
            walk_expr_flow(typed, rhs, flow, conventions, current_def_id);
        }

        TypedExprKind::FnCall { target, args } => {
            if let Some(args) = args {
                // Get param names for this target to map arg position to param name.
                let param_names: Vec<Intern<String>> = typed
                    .defs
                    .get(&target)
                    .map(|bind| bind.params.iter().map(|(pn, _)| *pn).collect())
                    .unwrap_or_default();

                for (arg_idx, arg_id) in args.iter().enumerate() {
                    let aidx = arg_id.as_usize();
                    let arg_kind = &typed.exprs.kind[aidx].clone();

                    // ConsumeArg: explicitly consumed via `~` at call site
                    if matches!(arg_kind, TypedExprKind::ConsumeArg(_)) {
                        walk_expr_flow(typed, *arg_id, flow, conventions, current_def_id);

                        // Check if the callee expects this param to be consumed.
                        // If the param is bare (Threaded), `~` at the call site
                        // is a contract mismatch.
                        let param_name = param_names.get(arg_idx);
                        let is_bare_param = param_name.is_some()
                            && conventions.get(&target).is_some_and(|conv| {
                                conv.conventions.get(param_name.unwrap())
                                    == Some(&InferredConvention::Threaded)
                            });

                        if is_bare_param {
                            add_flaw(
                                typed,
                                *arg_id,
                                TypeSymptom::ConsumeArgOnBareParam {
                                    name: param_name.unwrap().as_str().to_string(),
                                },
                            );
                        }

                        // Extract the inner variable and consume it in the
                        // caller's flow (honoring the `~` at the call site).
                        let inner_id = match arg_kind {
                            TypedExprKind::ConsumeArg(inner) => *inner,
                            _ => unreachable!(),
                        };
                        if let Some(name) =
                            resolve_expr_name_from_kind(&typed.exprs.kind[inner_id.as_usize()])
                            && flow.get_var_state(&name) == Some(VarState::Alive)
                        {
                            flow.set_var_state(name, VarState::Consumed);
                        }
                        continue;
                    }

                    // Bare arg: walk it
                    walk_expr_flow(typed, *arg_id, flow, conventions, current_def_id);

                    // Determine whether to consume based on convention
                    let should_consume = if let Some(param_name) = param_names.get(arg_idx) {
                        if let Some(conv) = conventions.get(&target) {
                            matches!(
                                conv.conventions.get(param_name),
                                Some(&InferredConvention::Consumed) | None
                            )
                        } else {
                            // No convention info — conservative: consume
                            true
                        }
                    } else {
                        true
                    };

                    if should_consume
                        && let Some(name) = resolve_expr_name_from_kind(&typed.exprs.kind[aidx])
                        && flow.get_var_state(&name) == Some(VarState::Alive)
                    {
                        flow.set_var_state(name, VarState::Consumed);
                    }
                    // If should_consume is false (Threaded), the variable stays Alive
                    // because it's threaded through the return.
                }
            }
        }

        TypedExprKind::TagCall { args, .. } => {
            if let Some(args) = args {
                for arg_id in &args {
                    walk_expr_flow(typed, *arg_id, flow, conventions, current_def_id);
                }
            }
        }

        TypedExprKind::Bind { name, stmts, body } => {
            for stmt in stmts {
                walk_expr_flow(typed, stmt, flow, conventions, current_def_id);
            }
            walk_expr_flow(typed, body, flow, conventions, current_def_id);
            flow.set_var_state(name, VarState::Alive);
        }

        TypedExprKind::When(ref when_expr) => {
            if let Some(subject) = when_expr.subject {
                walk_expr_flow(typed, subject, flow, conventions, current_def_id);
            }
            for arm in &when_expr.arms {
                match arm {
                    TypedWhenArm::Cond {
                        condition, body, ..
                    } => {
                        let mut cond_ctx = flow.clone();
                        walk_expr_flow(
                            typed,
                            *condition,
                            &mut cond_ctx,
                            conventions,
                            current_def_id,
                        );
                        let mut arm_ctx = flow.clone();
                        walk_expr_flow(typed, *body, &mut arm_ctx, conventions, current_def_id);
                    }
                    TypedWhenArm::Is { body, .. } => {
                        let mut arm_ctx = flow.clone();
                        walk_expr_flow(typed, *body, &mut arm_ctx, conventions, current_def_id);
                    }
                    TypedWhenArm::Else(body, _) => {
                        let mut else_ctx = flow.clone();
                        walk_expr_flow(typed, *body, &mut else_ctx, conventions, current_def_id);
                    }
                }
            }
        }

        TypedExprKind::If(ref if_expr) => {
            match if_expr.condition {
                TypedIfCondition::Bool(cond_id) => {
                    walk_expr_flow(typed, cond_id, flow, conventions, current_def_id);
                }
                TypedIfCondition::Pattern { subject, .. } => {
                    walk_expr_flow(typed, subject, flow, conventions, current_def_id);
                }
            }
            // Guard clause body — all statements execute then the return.
            let mut guard_ctx = flow.clone();
            for stmt in &if_expr.stmts {
                walk_expr_flow(typed, *stmt, &mut guard_ctx, conventions, current_def_id);
            }
            if let Some(ret) = if_expr.ret {
                walk_expr_flow(typed, ret, &mut guard_ctx, conventions, current_def_id);
            }
        }

        TypedExprKind::Loop(ref loop_expr) => {
            match loop_expr.kind {
                TypedLoopKind::While { condition } => {
                    walk_expr_flow(typed, condition, flow, conventions, current_def_id);
                }
                TypedLoopKind::ForIn { iterable, .. } => {
                    walk_expr_flow(typed, iterable, flow, conventions, current_def_id);
                }
            }
            let mut loop_ctx = FlowContext::with_parent(flow.clone());
            for stmt in &loop_expr.stmts {
                walk_expr_flow(typed, *stmt, &mut loop_ctx, conventions, current_def_id);
            }
        }

        TypedExprKind::SelfRef { .. } => {}

        TypedExprKind::FormatString(_) => {}

        TypedExprKind::Range { start, end } => {
            walk_expr_flow(typed, start, flow, conventions, current_def_id);
            walk_expr_flow(typed, end, flow, conventions, current_def_id);
        }

        TypedExprKind::TupleLit(items) | TypedExprKind::List(items) => {
            for id in &items {
                walk_expr_flow(typed, *id, flow, conventions, current_def_id);
            }
        }

        TypedExprKind::Cast { expr, .. } => {
            walk_expr_flow(typed, expr, flow, conventions, current_def_id);
        }

        TypedExprKind::TupleAlloc { init, .. } => {
            walk_expr_flow(typed, init, flow, conventions, current_def_id);
        }
        TypedExprKind::TupleGet { base, .. } => {
            walk_expr_flow(typed, base, flow, conventions, current_def_id);
        }
        TypedExprKind::TupleSet { base, value, .. } => {
            walk_expr_flow(typed, base, flow, conventions, current_def_id);
            walk_expr_flow(typed, value, flow, conventions, current_def_id);
        }

        TypedExprKind::BufGet { buf, index } => {
            walk_expr_flow(typed, buf, flow, conventions, current_def_id);
            walk_expr_flow(typed, index, flow, conventions, current_def_id);
            check_bounds(typed, buf, index, flow, expr_id);
        }
        TypedExprKind::BufSet { buf, index, value } => {
            walk_expr_flow(typed, buf, flow, conventions, current_def_id);
            walk_expr_flow(typed, index, flow, conventions, current_def_id);
            walk_expr_flow(typed, value, flow, conventions, current_def_id);
            check_bounds(typed, buf, index, flow, expr_id);
        }

        TypedExprKind::TakePtr(expr)
        | TypedExprKind::Ref(expr)
        | TypedExprKind::ConsumeArg(expr)
        | TypedExprKind::Deref(expr)
        | TypedExprKind::Negate(expr)
        | TypedExprKind::Eat(expr) => {
            walk_expr_flow(typed, expr, flow, conventions, current_def_id);
        }

        TypedExprKind::Asm(_) => {}
    }

    typed.exprs.flow[idx] = Some(flow.clone());
}

fn resolve_expr_name_from_kind(kind: &TypedExprKind) -> Option<Intern<String>> {
    match kind {
        TypedExprKind::Bind { name, .. } => Some(*name),
        TypedExprKind::FnCall { target, .. } => Some(target.0),
        _ => None,
    }
}

/// Check for values that are still alive at scope exit but should have been
/// consumed.
///
/// Checks local binds (`name: expr`) of non-Copy types — linearity requires
///    every non-Copy value to be consumed before scope exit.
///
/// Bare params are auto-threaded through the return — not checked.
/// Copy types (primitives, `and is Copy`) are silently dropped.
fn check_lin_values(
    typed: &mut TypedFileAst,
    flow: &FlowContext,
    consume_params: &std::collections::HashSet<Intern<String>>,
    local_var_types: &HashMap<Intern<String>, Ty>,
) {
    for (var_name, state) in flow.local_var_states() {
        if *state != VarState::Alive {
            continue;
        }

        // `~` (Consume) params are auto-dropped after their last usage, or
        // at scope exit if unused — no explicit consumption required.
        if consume_params.contains(var_name) {
            continue;
        }

        // For local binds, check if the type is non-Copy.
        if let Some(ty) = local_var_types.get(var_name) {
            if is_copyable(ty, &typed.marker_registry) {
                continue;
            }
        } else {
            // Variable not found in local binds — probably a bare param
            // (auto-threaded, not checked).
            continue;
        }

        // Infer consumption paths: look for methods on this type (by name).
        let consumption_paths = infer_consumption_paths(typed, var_name);

        // Attach the flaw to the last expression.
        if let Some(last_expr) = typed.exprs.kind.len().checked_sub(1) {
            typed.exprs.flaws[last_expr].push(TypeSymptom::LinValueNotConsumed {
                name: var_name.as_str().to_string(),
                consumption_paths,
            });
        }
    }
}

/// Infer consumption paths for a non-Copy type by scanning the typed AST for
/// methods whose receiver type matches the type name.
///
/// This is a best-effort heuristic — variable names often match type names.
/// A more precise implementation would track the actual type of each variable
/// through the expression arena.
fn infer_consumption_paths(typed: &TypedFileAst, type_name: &Intern<String>) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();

    for (def_id, bind) in &typed.defs {
        // Check if this bind is a method on the given type
        if let Some(ref receiver_ty) = bind.receiver_type {
            match receiver_ty {
                Ty::Record { name, .. } | Ty::Union { name, .. } | Ty::Opaque(name)
                    if name == type_name =>
                {
                    paths.push(format!("{}(own self)", def_id.0.as_str()));
                }
                _ => {}
            }
        }
    }

    paths.sort();
    paths
}

/// Check if a constant index is out of bounds for an array/buffer access.
fn check_bounds(
    typed: &mut TypedFileAst,
    base: ExprId,
    index: ExprId,
    _flow: &FlowContext,
    expr_id: ExprId,
) {
    let base_idx = base.as_usize();
    let index_idx = index.as_usize();
    if base_idx >= typed.exprs.kind.len() || index_idx >= typed.exprs.kind.len() {
        return;
    }

    let const_index = match &typed.exprs.kind[index_idx] {
        TypedExprKind::Lit(Literal::Int(n)) => Some(*n as i128),
        TypedExprKind::Lit(Literal::Number(n)) => Some(*n as i128),
        _ => None,
    };

    let Some(const_index) = const_index else {
        return;
    };

    let base_ty = &typed.exprs.ty[base_idx];
    let size = match base_ty {
        Ty::Array { size, .. } if *size > 0 => Some(*size),
        _ => None,
    };

    let Some(size) = size else {
        return;
    };

    if const_index < 0 || const_index as usize >= size {
        add_flaw(
            typed,
            expr_id,
            TypeSymptom::IndexOutOfBounds {
                index: const_index,
                size,
            },
        );
    }
}

fn add_flaw(typed: &mut TypedFileAst, expr_id: ExprId, flaw: TypeSymptom) {
    let idx = expr_id.as_usize();
    if idx < typed.exprs.flaws.len() {
        typed.exprs.flaws[idx].push(flaw);
    }
}
