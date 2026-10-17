use std::collections::{HashMap, HashSet};

use ast::NormalExpr;

use crate::typed::{
    Availability, CallCapability, DefId, ExprId, FunctionEffects, TypedExprKind, TypedFileAst,
    TypedWhenArm, walk_expr_children, walk_expr_preorder,
};

use super::CallableSignatures;
use crate::transform::finish_reads;

#[derive(Clone)]
enum VisitState {
    Unvisited,
    Visiting,
    Done,
}

struct AvailabilityState<'a> {
    typed: &'a TypedFileAst,
    signatures: &'a CallableSignatures,
    state: &'a mut [VisitState],
    bind_cache: &'a mut HashMap<DefId, Availability>,
    out: &'a mut [Availability],
    compiletime_refs: &'a HashSet<ExprId>,
}

/// Fill `expr.availability` for all lowered expressions.
pub fn analyze_availability(typed: &mut TypedFileAst, signatures: &CallableSignatures) {
    let _profile = compiler_profile::span(compiler_profile::Phase::Staging);
    let len = typed.exprs.kind.len();
    let mut state = vec![VisitState::Unvisited; len];
    let mut bind_cache: HashMap<DefId, Availability> = HashMap::new();
    let mut compiletime_refs = HashSet::new();
    for kind in &typed.exprs.kind {
        if let TypedExprKind::When(when_expr) = kind {
            for arm in &when_expr.arms {
                if let TypedWhenArm::Is { body, .. } = arm {
                    mark_pattern_exprs(*body, typed, &mut compiletime_refs);
                }
            }
        }
    }
    let mut avail = vec![Availability::Unknown; len];
    for idx in 0..len {
        let expr_id = ExprId(idx as u32);
        let availability = analyze_expr(
            expr_id,
            &mut AvailabilityState {
                typed,
                signatures,
                state: &mut state,
                bind_cache: &mut bind_cache,
                out: &mut avail,
                compiletime_refs: &compiletime_refs,
            },
        );
        avail[idx] = availability;
    }

    typed.exprs.facts.semantic_facts.availability[..len].copy_from_slice(&avail[..len]);
}

fn analyze_expr(expr_id: ExprId, ctx: &mut AvailabilityState<'_>) -> Availability {
    let idx = expr_id.as_usize();
    match ctx.state.get(idx) {
        Some(VisitState::Done) => return ctx.out[idx],
        Some(VisitState::Visiting) => return Availability::Runtime,
        Some(VisitState::Unvisited) => {}
        None => return Availability::Unknown,
    }

    if !ctx.typed.exprs.facts.semantic_facts.flaws[idx].is_empty() {
        ctx.state[idx] = VisitState::Done;
        ctx.out[idx] = Availability::Unknown;
        return Availability::Unknown;
    }

    ctx.state[idx] = VisitState::Visiting;
    if let Some(version) = ctx.typed.exprs.place_version_of(expr_id).flatten()
        && matches!(
            ctx.typed.exprs.kind[idx],
            TypedExprKind::FnCall { args: None, .. }
        )
        && let crate::typed::PlaceVersionOrigin::Initializer(value) =
            ctx.typed.place_versions[version.0 as usize].origin
    {
        let availability = analyze_expr(value, ctx);
        ctx.state[idx] = VisitState::Done;
        ctx.out[idx] = availability;
        return availability;
    }
    let kind = &ctx.typed.exprs.kind[idx];
    let availability = match kind {
        TypedExprKind::Block { statements, value } => {
            join_exprs(ctx, statements.iter().chain(value).copied())
        }
        TypedExprKind::Return { value, .. } => {
            value.map_or(Availability::CompileTime, |child| analyze_expr(child, ctx))
        }
        TypedExprKind::Invalid => Availability::Unknown,
        TypedExprKind::Todo => Availability::CompileTime,
        TypedExprKind::Emit { .. } => Availability::Runtime,
        TypedExprKind::Domain(_)
        | TypedExprKind::Lit(_)
        | TypedExprKind::Rematerialize { .. }
        | TypedExprKind::TargetQuery { .. }
        | TypedExprKind::Target => Availability::CompileTime,
        TypedExprKind::TargetField(_) => Availability::CompileTime,
        TypedExprKind::Binary { lhs, rhs, .. }
        | TypedExprKind::InvalidOperator { lhs, rhs, .. } => join_exprs(ctx, [*lhs, *rhs]),
        TypedExprKind::IntrinsicCall { args, .. } => join_exprs(ctx, args.iter().copied()),
        TypedExprKind::FnCall { target, args, .. }
            if args.as_ref().is_none_or(Vec::is_empty)
                && !ctx.typed.defs.contains_key(target)
                && !ctx.signatures.contains_key(target)
                && ctx.compiletime_refs.contains(&expr_id) =>
        {
            finish_reads::record_signature(*target, ctx.signatures.get(target));
            Availability::CompileTime
        }
        TypedExprKind::FnCall { target, args, .. } => analyze_call(*target, args.as_deref(), ctx),
        TypedExprKind::TagCall {
            args: Some(args), ..
        } => join_exprs(ctx, args.iter().copied()),
        TypedExprKind::TagCall { args: None, .. } => Availability::CompileTime,
        TypedExprKind::Bind {
            stmts,
            body,
            unassigned,
            ..
        } => join_exprs(
            ctx,
            stmts.iter().copied().chain((!unassigned).then_some(*body)),
        ),
        TypedExprKind::Reassign { .. } => Availability::Runtime,
        TypedExprKind::When(when_expr) => {
            let mut children = Vec::new();
            if let Some(subject) = when_expr.subject {
                children.push(subject);
            }
            for arm in &when_expr.arms {
                match arm {
                    TypedWhenArm::Cond {
                        condition, body, ..
                    } => {
                        condition.visit_subjects(&mut |subject| children.push(subject));
                        children.push(*body);
                    }
                    TypedWhenArm::Is { body, .. } | TypedWhenArm::Else(body, _) => {
                        children.push(*body);
                    }
                }
            }
            join_exprs(ctx, children)
        }
        TypedExprKind::If { .. } => {
            let mut exprs = Vec::new();
            let _ = walk_expr_children(kind, &mut |child| {
                exprs.push(child);
                std::ops::ControlFlow::Continue(())
            });
            join_exprs(ctx, exprs)
        }
        TypedExprKind::Loop { .. } => Availability::Runtime,
        TypedExprKind::SelfRef { .. } => Availability::CompileTime,
        TypedExprKind::Range { start, end } => join_exprs(ctx, [*start, *end]),
        TypedExprKind::RecordValue(values)
        | TypedExprKind::ResultRow(values)
        | TypedExprKind::ArrayLit(values)
        | TypedExprKind::List(values) => join_exprs(ctx, values.iter().copied()),
        TypedExprKind::Cast { expr, .. } | TypedExprKind::UnionPayload { expr, .. } => {
            analyze_expr(*expr, ctx)
        }
        TypedExprKind::FixedArrayAlloc { init, size } => {
            let init_availability = analyze_expr(*init, ctx);
            let size_is_const = matches!(size, NormalExpr::Value(_));
            if init_availability == Availability::CompileTime && size_is_const {
                Availability::CompileTime
            } else {
                Availability::Runtime
            }
        }
        TypedExprKind::AggregateGet { base, .. } => analyze_expr(*base, ctx),
        TypedExprKind::AggregateSet { .. } => Availability::Runtime,
        TypedExprKind::Destructure { .. } => Availability::Runtime,
        TypedExprKind::RecordSet { .. } => Availability::Runtime,
        TypedExprKind::SliceGet { buf, start, length } => join_exprs(ctx, [*buf, *start, *length]),
        TypedExprKind::BufGet { buf, index } => join_exprs(ctx, [*buf, *index]),
        TypedExprKind::BufSet { .. } => Availability::Runtime,
        TypedExprKind::TakePtr(inner) => analyze_expr(*inner, ctx),
        TypedExprKind::Ref(expr) | TypedExprKind::Deref(expr) | TypedExprKind::Negate(expr) => {
            analyze_expr(*expr, ctx)
        }
    };
    ctx.state[idx] = VisitState::Done;
    ctx.out[idx] = availability;
    availability
}

fn analyze_call(
    target: DefId,
    args: Option<&[ExprId]>,
    availability_state: &mut AvailabilityState<'_>,
) -> Availability {
    let typed = availability_state.typed;
    let signatures = availability_state.signatures;
    if target.0.as_str() == "Self" {
        return Availability::CompileTime;
    }

    let callee_availability =
        bind_availability(target, typed, signatures, availability_state.bind_cache);
    let args_availability = args.map_or(Availability::CompileTime, |call_args| {
        join_exprs(availability_state, call_args.iter().copied())
    });
    join_availability(&[callee_availability, args_availability])
}

fn bind_availability(
    target: DefId,
    typed: &TypedFileAst,
    signatures: &CallableSignatures,
    cache: &mut HashMap<DefId, Availability>,
) -> Availability {
    if let Some(cached) = cache.get(&target) {
        return *cached;
    }

    let result = if let Some(bind) = typed.defs.get(&target) {
        if !bind.flaws.is_empty() {
            Availability::Unknown
        } else if has_runtime_effects(&bind.effects) {
            Availability::Runtime
        } else {
            match bind.call_capability {
                CallCapability::StagePolymorphic => Availability::CompileTime,
                CallCapability::RuntimeOnly => Availability::Runtime,
            }
        }
    } else if let Some(signature) = signatures.get(&target) {
        finish_reads::record_signature(target, Some(signature));
        if has_runtime_effects(&signature.effects) {
            Availability::Runtime
        } else {
            match signature.call_capability {
                CallCapability::StagePolymorphic => Availability::CompileTime,
                CallCapability::RuntimeOnly => Availability::Runtime,
            }
        }
    } else {
        finish_reads::record_signature(target, None);
        Availability::Runtime
    };
    cache.insert(target, result);
    result
}

fn has_runtime_effects(effects: &FunctionEffects) -> bool {
    !effects.reads.is_empty()
        || !effects.writes.is_empty()
        || !effects.invalidates.is_empty()
        || !effects.consumes.is_empty()
}

fn mark_pattern_exprs(root: ExprId, typed: &TypedFileAst, marked: &mut HashSet<ExprId>) {
    let _ = walk_expr_preorder(typed, root, &mut |expr_id| {
        marked.insert(expr_id);
        std::ops::ControlFlow::Continue(())
    });
}

fn join_exprs(
    ctx: &mut AvailabilityState<'_>,
    exprs: impl IntoIterator<Item = ExprId>,
) -> Availability {
    let availabilities = exprs
        .into_iter()
        .map(|expr| analyze_expr(expr, ctx))
        .collect::<Vec<_>>();
    join_availability(&availabilities)
}

fn join_availability(availabilities: &[Availability]) -> Availability {
    if availabilities.is_empty() {
        return Availability::CompileTime;
    }
    if availabilities
        .iter()
        .any(|a| matches!(a, Availability::Runtime))
    {
        Availability::Runtime
    } else if availabilities
        .iter()
        .all(|a| matches!(a, Availability::CompileTime))
    {
        Availability::CompileTime
    } else {
        Availability::Unknown
    }
}
