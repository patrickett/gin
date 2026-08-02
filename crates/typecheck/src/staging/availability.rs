use std::collections::{HashMap, HashSet};

use ast::NormalExpr;

use crate::typed::{
    Availability, CallCapability, DefId, ExprId, FunctionEffects, TypedExprKind, TypedFileAst,
    TypedWhenArm, walk_expr_children,
};

use super::CallableSignatures;

#[derive(Clone)]
enum VisitState {
    Unvisited,
    Visiting,
    Done,
}

struct AvailabilityState<'a> {
    state: &'a mut [VisitState],
    bind_cache: &'a mut HashMap<DefId, Availability>,
    out: &'a mut [Availability],
    compiletime_refs: &'a HashSet<ExprId>,
}

/// Fill `expr.availability` for all lowered expressions.
pub fn analyze_availability(typed: &mut TypedFileAst, signatures: &CallableSignatures) {
    let len = typed.exprs.kind.len();
    let mut state = vec![VisitState::Unvisited; len];
    let mut bind_cache: HashMap<DefId, Availability> = HashMap::new();
    let mut compiletime_refs = HashSet::new();
    for kind in &typed.exprs.kind {
        if let TypedExprKind::When(when_expr) = kind {
            if let Some(subject) = when_expr.subject
                && matches!(&typed.exprs.ty[subject.as_usize()], crate::ty::Ty::Opaque(name) if name.as_str() == "Type")
            {
                compiletime_refs.insert(subject);
            }
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
        avail[idx] = analyze_expr(
            expr_id,
            typed,
            signatures,
            &mut state,
            &mut bind_cache,
            &mut avail,
            &compiletime_refs,
        );
    }

    typed.exprs.availability[..len].copy_from_slice(&avail[..len]);
}

fn analyze_expr(
    expr_id: ExprId,
    typed: &TypedFileAst,
    signatures: &CallableSignatures,
    state: &mut [VisitState],
    bind_cache: &mut HashMap<DefId, Availability>,
    out: &mut [Availability],
    compiletime_refs: &HashSet<ExprId>,
) -> Availability {
    let idx = expr_id.as_usize();
    match state.get(idx) {
        Some(VisitState::Done) => return out[idx],
        Some(VisitState::Visiting) => return Availability::Runtime,
        Some(VisitState::Unvisited) => {}
        None => return Availability::Unknown,
    }

    if !typed.exprs.flaws[idx].is_empty() {
        state[idx] = VisitState::Done;
        out[idx] = Availability::Unknown;
        return Availability::Unknown;
    }

    state[idx] = VisitState::Visiting;
    let kind = &typed.exprs.kind[idx];
    let availability = match kind {
        TypedExprKind::Lit(_) => Availability::CompileTime,
        TypedExprKind::Binary { lhs, rhs, .. } => join_availability(&[
            analyze_expr(*lhs, typed, signatures, state, bind_cache, out, compiletime_refs),
            analyze_expr(*rhs, typed, signatures, state, bind_cache, out, compiletime_refs),
        ]),
        TypedExprKind::FnCall { target, args, .. }
            if args.as_ref().is_none_or(Vec::is_empty)
                && !typed.defs.contains_key(target)
                && !signatures.contains_key(target)
                && compiletime_refs.contains(&expr_id) => Availability::CompileTime,
        TypedExprKind::FnCall { target, args, .. } => analyze_call(
            *target,
            args.as_deref(),
            typed,
            signatures,
            &mut AvailabilityState {
                state,
                bind_cache,
                out,
                compiletime_refs,
            },
        ),
        TypedExprKind::TagCall {
            args: Some(args), ..
        } => join_availability(
            &args
                .iter()
                .map(|arg| analyze_expr(*arg, typed, signatures, state, bind_cache, out, compiletime_refs))
                .collect::<Vec<_>>(),
        ),
        TypedExprKind::TagCall { args: None, .. } => Availability::CompileTime,
        TypedExprKind::Bind {
            body, unassigned, ..
        } => {
            if *unassigned || body.0 == 0 {
                Availability::CompileTime
            } else {
                analyze_expr(*body, typed, signatures, state, bind_cache, out, compiletime_refs)
            }
        }
        TypedExprKind::Reassign { .. } => Availability::Runtime,
        TypedExprKind::When(when_expr) => {
            let mut children = Vec::new();
            if let Some(subject) = when_expr.subject {
                children.push(subject);
            }
            for arm in &when_expr.arms {
                match arm {
                    TypedWhenArm::Cond { condition, body, .. } => {
                        children.push(*condition);
                        children.push(*body);
                    }
                    TypedWhenArm::Is { body, .. } | TypedWhenArm::Else(body, _) => {
                        children.push(*body);
                    }
                }
            }
            join_availability(
                &children
                    .into_iter()
                    .map(|child| analyze_expr(child, typed, signatures, state, bind_cache, out, compiletime_refs))
                    .collect::<Vec<_>>(),
            )
        }
        TypedExprKind::If { .. } => {
            let exprs = walk_if_children(kind);
            join_availability(
                &exprs
                    .into_iter()
                    .map(|child| analyze_expr(child, typed, signatures, state, bind_cache, out, compiletime_refs))
                    .collect::<Vec<_>>(),
            )
        }
        TypedExprKind::Loop { .. } => Availability::Runtime,
        TypedExprKind::SelfRef { .. } => Availability::CompileTime,
        TypedExprKind::FormatString(_) => Availability::CompileTime,
        TypedExprKind::Range { start, end } => join_availability(&[
            analyze_expr(*start, typed, signatures, state, bind_cache, out, compiletime_refs),
            analyze_expr(*end, typed, signatures, state, bind_cache, out, compiletime_refs),
        ]),
        TypedExprKind::TupleLit(values) | TypedExprKind::List(values) => join_availability(
            &values
                .iter()
                .map(|value| analyze_expr(*value, typed, signatures, state, bind_cache, out, compiletime_refs))
                .collect::<Vec<_>>(),
        ),
        TypedExprKind::Cast { expr, .. } => {
            analyze_expr(*expr, typed, signatures, state, bind_cache, out, compiletime_refs)
        }
        TypedExprKind::TupleAlloc { init, size } => {
            let init_availability = analyze_expr(*init, typed, signatures, state, bind_cache, out, compiletime_refs);
            let size_is_const = matches!(size, NormalExpr::Value(_));
            if init_availability == Availability::CompileTime && size_is_const {
                Availability::CompileTime
            } else {
                Availability::Runtime
            }
        }
        TypedExprKind::TupleGet { base, .. } => {
            analyze_expr(*base, typed, signatures, state, bind_cache, out, compiletime_refs)
        }
        TypedExprKind::TupleSet { .. } => Availability::Runtime,
        TypedExprKind::Destructure { .. } => Availability::Runtime,
        TypedExprKind::RecordSet { .. } => Availability::Runtime,
        TypedExprKind::BufGet { buf, index } => join_availability(&[
            analyze_expr(*buf, typed, signatures, state, bind_cache, out, compiletime_refs),
            analyze_expr(*index, typed, signatures, state, bind_cache, out, compiletime_refs),
        ]),
        TypedExprKind::BufSet { .. } => Availability::Runtime,
        TypedExprKind::TakePtr(inner) => {
            analyze_expr(*inner, typed, signatures, state, bind_cache, out, compiletime_refs)
        }
        TypedExprKind::Ref(expr) | TypedExprKind::Deref(expr) | TypedExprKind::Negate(expr) => {
            analyze_expr(*expr, typed, signatures, state, bind_cache, out, compiletime_refs)
        }
        TypedExprKind::ConsumeArg(expr) | TypedExprKind::Eat(expr) => {
            analyze_expr(*expr, typed, signatures, state, bind_cache, out, compiletime_refs)
        }
        TypedExprKind::Asm(_) => Availability::Runtime,
    };
    state[idx] = VisitState::Done;
    out[idx] = availability;
    availability
}

fn analyze_call(
    target: DefId,
    args: Option<&[ExprId]>,
    typed: &TypedFileAst,
    signatures: &CallableSignatures,
    availability_state: &mut AvailabilityState<'_>,
) -> Availability {
    if target.0.as_str() == "asm" {
        return Availability::Runtime;
    }
    if target.0.as_str() == "Self" {
        return Availability::CompileTime;
    }

    let callee_availability = bind_availability(
        target,
        typed,
        signatures,
        availability_state.bind_cache,
    );
    let arg_availabilities = args
        .map(|call_args| {
            call_args
                .iter()
                .map(|arg| {
                    analyze_expr(
                        *arg,
                        typed,
                        signatures,
                        availability_state.state,
                        availability_state.bind_cache,
                        availability_state.out,
                        availability_state.compiletime_refs,
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if matches!(callee_availability, Availability::Runtime) {
        return Availability::Runtime;
    }
    if arg_availabilities.contains(&Availability::Runtime) {
        return Availability::Runtime;
    }
    join_availability(&arg_availabilities)
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
        if has_runtime_effects(&signature.effects) {
            Availability::Runtime
        } else {
            match signature.call_capability {
                CallCapability::StagePolymorphic => Availability::CompileTime,
                CallCapability::RuntimeOnly => Availability::Runtime,
            }
        }
    } else {
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
    let mut pending = vec![root];
    while let Some(expr_id) = pending.pop() {
        if !marked.insert(expr_id) {
            continue;
        }
        let _ = walk_expr_children(&typed.exprs.kind[expr_id.as_usize()], &mut |child| {
            pending.push(child);
            std::ops::ControlFlow::Continue(())
        });
    }
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

fn walk_if_children(kind: &TypedExprKind) -> Vec<ExprId> {
    let TypedExprKind::If(if_expr) = kind else {
        return Vec::new();
    };
    let mut children = Vec::with_capacity(if_expr.stmts.len() + 2);
    children.push(if_expr.subject);
    children.extend(if_expr.stmts.iter().copied());
    if let Some(ret) = if_expr.ret {
        children.push(ret);
    }
    children
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::{TransformCtx, transform};
    use crate::typed::{BindBody, TypedExprKind};
    use crate::{FileId, prepare_parse_ast};
    use flask::CompileTarget;
    use internment::Intern;
    use parser::cursor::TokenCursor;

    fn transform_prepared(source: &str) -> crate::TypedFileAst {
        let mut file_ast = TokenCursor::parse_source(source);
        let _ = prepare_parse_ast(&mut file_ast, &CompileTarget::Library);
        transform(&file_ast, FileId(0), &TransformCtx::new())
    }

    fn return_expr(typed: &crate::TypedFileAst, name: &str) -> crate::ExprId {
        let bind = typed
            .defs
            .get(&crate::DefId(Intern::from_ref(name)))
            .expect("definition");
        match &bind.body {
            BindBody::Expr(expr) => *expr,
            BindBody::Body {
                ret: Some(expr), ..
            } => *expr,
            _ => panic!("definition has no return expression"),
        }
    }

    #[test]
    fn availability_join() {
        use Availability::*;
        assert_eq!(join_availability(&[CompileTime, CompileTime]), CompileTime);
        assert_eq!(join_availability(&[CompileTime, Runtime]), Runtime);
        assert_eq!(join_availability(&[Runtime, Unknown]), Runtime);
        assert_eq!(join_availability(&[Unknown, CompileTime]), Unknown);
        assert_eq!(join_availability(&[]), CompileTime);
    }

    #[test]
    fn literal_binary_expression_is_compile_time() {
        let typed = transform_prepared("main: 1 + 2\n");
        let root = return_expr(&typed, "main");

        assert!(matches!(
            typed.exprs.kind[root.as_usize()],
            TypedExprKind::Binary { .. }
        ));
        assert_eq!(
            typed.exprs.availability[root.as_usize()],
            Availability::CompileTime
        );
    }

    #[test]
    fn runtime_parameter_forces_runtime_expression() {
        let typed = transform_prepared("main(x) Int: x + 2\n");
        let root = return_expr(&typed, "main");

        assert_eq!(
            typed.exprs.availability[root.as_usize()],
            Availability::Runtime
        );
    }

    #[test]
    fn constructor_from_compile_time_fields_is_compile_time() {
        let typed = transform_prepared("Maybe(x) is Some(x) or None\nmain: Maybe.Some(1)\n");
        let root = return_expr(&typed, "main");
        assert_eq!(
            typed.exprs.availability[root.as_usize()],
            Availability::CompileTime
        );
    }

    #[test]
    fn asm_inference_is_runtime() {
        let typed = transform_prepared(
            "spec := 'svc #0x80'\n\
             write(fd Int, buf Int, len Int) Int: asm(spec, fd, buf, len)\n\
             main: write(1, 2, 3)\n",
        );
        let write_root = return_expr(&typed, "write");
        let main_root = return_expr(&typed, "main");

        assert_eq!(
            typed.exprs.availability[write_root.as_usize()],
            Availability::Runtime
        );
        assert_eq!(
            typed.exprs.availability[main_root.as_usize()],
            Availability::Runtime
        );
    }

    #[test]
    fn flaw_free_exprs_have_non_unknown_availability() {
        let typed = transform_prepared("main: 1 + 2\n");
        for index in 0..typed.exprs.availability.len() {
            if typed.exprs.flaws[index].is_empty() {
                assert_ne!(typed.exprs.availability[index], Availability::Unknown);
            }
        }
    }
}
