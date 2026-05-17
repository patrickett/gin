//! Stage 3: Flow — Compute flow contexts, attach flow flaws.
//!
//! This stage walks the typed expression arena depth-first, computes
//! `FlowContext` at each program point, and attaches flow-related flaws
//! (ownership, bounds, narrowing, lin-value checks, etc.).

use crate::analysis::{Capability, FlowContext, VarState};
use crate::expr::Literal;
use crate::ty::Ty;
use crate::typed::{
    ExprId, TypedExprKind, TypedFileAst, TypedIfCondition, TypedLoopKind, TypedWhenArm,
};
use diagnostic::TypeSymptom;
use internment::Intern;

/// Walks the expression arena, computes flow contexts at each program point,
/// and attaches flow-related flaws to expressions.
pub fn stage_flow(typed: &mut TypedFileAst) {
    // Build a set of lin type names for LinValueNotConsumed checks.
    let lin_types: std::collections::HashSet<Intern<String>> = typed
        .tags
        .iter()
        .filter(|(_, tag)| tag.attributes.is_lin)
        .map(|(tag_id, _)| tag_id.0)
        .collect();

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
    for (_def_id, root_ids) in &bind_bodies {
        let mut flow = FlowContext::new();
        for root_id in root_ids {
            walk_expr_flow(typed, *root_id, &mut flow);
        }
        // Check for unconsumed lin values at scope exit.
        check_lin_values(typed, &flow, &lin_types);
    }

    // Walk top-level expressions.
    let mut flow = FlowContext::new();
    for root_id in typed.root_exprs.clone() {
        walk_expr_flow(typed, root_id, &mut flow);
    }
}

fn walk_expr_flow(typed: &mut TypedFileAst, expr_id: ExprId, flow: &mut FlowContext) {
    let idx = expr_id.as_usize();
    if idx >= typed.exprs.kind.len() {
        return;
    }

    // Record the flow context at this program point.
    typed.exprs.flow[idx] = Some(flow.clone());

    let kind = typed.exprs.kind[idx].clone();

    // Check for use-after-move: if this expression references a moved variable.
    if let Some(name) = resolve_expr_name_from_kind(&kind)
        && flow.get_var_state(&name) == Some(VarState::Moved)
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
            walk_expr_flow(typed, lhs, flow);
            walk_expr_flow(typed, rhs, flow);
        }

        TypedExprKind::FnCall { args, .. } => {
            if let Some(args) = args {
                for arg_id in &args {
                    walk_expr_flow(typed, *arg_id, flow);
                }
            }
        }

        TypedExprKind::TagCall { args, .. } => {
            if let Some(args) = args {
                for arg_id in &args {
                    walk_expr_flow(typed, *arg_id, flow);
                }
            }
        }

        TypedExprKind::Bind { name, body } => {
            walk_expr_flow(typed, body, flow);
            flow.set_var_state(name, VarState::Alive);
            flow.set_capability(name, Capability::Own);
        }

        TypedExprKind::When(ref when_expr) => {
            if let Some(subject) = when_expr.subject {
                walk_expr_flow(typed, subject, flow);
            }
            for arm in &when_expr.arms {
                match arm {
                    TypedWhenArm::Cond {
                        condition, body, ..
                    } => {
                        let mut cond_ctx = flow.clone();
                        walk_expr_flow(typed, *condition, &mut cond_ctx);
                        let mut arm_ctx = flow.clone();
                        walk_expr_flow(typed, *body, &mut arm_ctx);
                    }
                    TypedWhenArm::Is { body, .. } => {
                        let mut arm_ctx = flow.clone();
                        walk_expr_flow(typed, *body, &mut arm_ctx);
                    }
                    TypedWhenArm::Else(body, _) => {
                        let mut else_ctx = flow.clone();
                        walk_expr_flow(typed, *body, &mut else_ctx);
                    }
                }
            }
        }

        TypedExprKind::If(ref if_expr) => {
            match if_expr.condition {
                TypedIfCondition::Bool(cond_id) => {
                    walk_expr_flow(typed, cond_id, flow);
                }
                TypedIfCondition::Pattern { subject, .. } => {
                    walk_expr_flow(typed, subject, flow);
                }
            }
            let mut then_ctx = flow.clone();
            walk_expr_flow(typed, if_expr.then_body, &mut then_ctx);
            if let Some(else_body) = if_expr.else_body {
                let mut else_ctx = flow.clone();
                walk_expr_flow(typed, else_body, &mut else_ctx);
            }
        }

        TypedExprKind::Loop(ref loop_expr) => {
            match loop_expr.kind {
                TypedLoopKind::While { condition } => {
                    walk_expr_flow(typed, condition, flow);
                }
                TypedLoopKind::ForIn { iterable, .. } => {
                    walk_expr_flow(typed, iterable, flow);
                }
            }
            let mut loop_ctx = FlowContext::with_parent(flow.clone());
            walk_expr_flow(typed, loop_expr.body, &mut loop_ctx);
        }

        TypedExprKind::SelfRef { .. } => {}

        TypedExprKind::FormatString(_) => {}

        TypedExprKind::Range { start, end } => {
            walk_expr_flow(typed, start, flow);
            walk_expr_flow(typed, end, flow);
        }

        TypedExprKind::TupleLit(items) | TypedExprKind::List(items) => {
            for id in &items {
                walk_expr_flow(typed, *id, flow);
            }
        }

        TypedExprKind::Cast { expr, .. } => {
            walk_expr_flow(typed, expr, flow);
        }

        TypedExprKind::TupleAlloc { init, .. } => {
            walk_expr_flow(typed, init, flow);
        }
        TypedExprKind::TupleGet { base, .. } => {
            walk_expr_flow(typed, base, flow);
        }
        TypedExprKind::TupleSet { base, value, .. } => {
            walk_expr_flow(typed, base, flow);
            walk_expr_flow(typed, value, flow);
        }

        TypedExprKind::BufGet { buf, index } => {
            walk_expr_flow(typed, buf, flow);
            walk_expr_flow(typed, index, flow);
            check_bounds(typed, buf, index, flow, expr_id);
        }
        TypedExprKind::BufSet { buf, index, value } => {
            walk_expr_flow(typed, buf, flow);
            walk_expr_flow(typed, index, flow);
            walk_expr_flow(typed, value, flow);
            check_bounds(typed, buf, index, flow, expr_id);
        }

        TypedExprKind::TakePtr(expr)
        | TypedExprKind::TakeRef(expr)
        | TypedExprKind::Deref(expr)
        | TypedExprKind::Negate(expr) => {
            walk_expr_flow(typed, expr, flow);
        }

        TypedExprKind::MutArg(expr) => {
            walk_expr_flow(typed, expr, flow);
            if let Some(name) = resolve_expr_name_from_kind(&typed.exprs.kind[expr.as_usize()]) {
                let cap = flow.get_capability(&name).unwrap_or(Capability::Read);
                if cap < Capability::Write {
                    add_flaw(
                        typed,
                        expr_id,
                        TypeSymptom::CannotPassReadonlyAsMut {
                            name: name.as_str().to_string(),
                        },
                    );
                }
            }
        }

        TypedExprKind::OwnArg(expr) => {
            walk_expr_flow(typed, expr, flow);
            if let Some(name) = resolve_expr_name_from_kind(&typed.exprs.kind[expr.as_usize()])
                && flow.get_var_state(&name) == Some(VarState::Alive)
            {
                flow.set_var_state(name, VarState::Moved);
            }
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

/// Check for lin values that are alive at scope exit but not consumed.
fn check_lin_values(
    typed: &mut TypedFileAst,
    flow: &FlowContext,
    lin_types: &std::collections::HashSet<Intern<String>>,
) {
    for (var_name, state) in flow.local_var_states() {
        if *state != VarState::Alive {
            continue;
        }
        // Check if this variable has a lin type by looking up its type in the arena.
        // For now, we check if the name appears in the tags as a lin type.
        if lin_types.contains(var_name) {
            // We need to find the expression that corresponds to this variable's point of creation
            // to attach the flaw. For simplicity, attach to the last expression.
            if let Some(last_expr) = typed.exprs.kind.len().checked_sub(1) {
                typed.exprs.flaws[last_expr].push(TypeSymptom::LinValueNotConsumed {
                    name: var_name.as_str().to_string(),
                });
            }
        }
    }
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
