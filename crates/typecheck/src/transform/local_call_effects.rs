use std::collections::HashSet;
use std::ops::ControlFlow;

use ast::ParamConvention;

use crate::typed::{DefId, ExprId, PlaceId, TypedCallableSignature, TypedExprKind, TypedFileAst};

pub(super) fn may_write(signature: &TypedCallableSignature) -> bool {
    signature
        .param_conventions
        .contains(&ParamConvention::Mutate)
        || signature.effects.unknown_memory
        || !signature.effects.writes.is_empty()
        || !signature.effects.scalar_fields.is_empty()
        || !signature.effects.invalidates.is_empty()
}

pub(super) fn written_places(
    typed: &TypedFileAst,
    callable: ExprId,
    may_write_call: impl Fn(DefId) -> bool,
) -> Option<HashSet<PlaceId>> {
    let mut callables = vec![callable];
    let mut visited_callables = HashSet::new();
    let mut writes = HashSet::new();
    while let Some(callable) = callables.pop() {
        if !visited_callables.insert(callable) {
            continue;
        }
        let parameters = typed.local_callable_parameters.get(&callable)?;
        let TypedExprKind::Bind {
            stmts,
            body,
            unassigned: false,
            ..
        } = typed.exprs.kind_of(callable)?
        else {
            return None;
        };
        let mut pending: Vec<_> = stmts.iter().copied().chain([*body]).collect();
        let mut visited = HashSet::new();
        while let Some(expression) = pending.pop() {
            if !visited.insert(expression)
                || typed.local_callable_parameters.contains_key(&expression)
            {
                continue;
            }
            let kind = typed.exprs.kind_of(expression)?;
            if let Some(callee) = typed.local_callable_calls.get(&expression) {
                callables.push(*callee);
            } else if let TypedExprKind::FnCall { target, args, .. } = kind {
                let value_read = args.is_none()
                    && (typed.exprs.place_of(expression).flatten().is_some()
                        || parameters
                            .iter()
                            .any(|parameter| parameter.name == target.0));
                if !value_read && may_write_call(*target) {
                    return None;
                }
            }
            if matches!(
                kind,
                TypedExprKind::Reassign { .. }
                    | TypedExprKind::RecordSet { .. }
                    | TypedExprKind::AggregateSet { .. }
                    | TypedExprKind::BufSet { .. }
            ) {
                let mut place = typed.exprs.place_of(expression).flatten()?;
                while let Some(parent) = typed.places[place.0 as usize].parent {
                    place = parent;
                }
                let ty = &typed.places[place.0 as usize].ty;
                if crate::ty::reference_semantics_for_type(ty, Some(&typed.type_registry)).is_some()
                    || typed
                        .type_registry
                        .resolved_definition_for_type(ty)
                        .pointee_ty()
                        .is_some()
                {
                    return None;
                }
                writes.insert(place);
            }
            if matches!(
                kind,
                TypedExprKind::Emit { .. }
                    | TypedExprKind::IntrinsicCall {
                        op: crate::intrinsic::IntrinsicOp::AddressStore,
                        ..
                    }
            ) {
                return None;
            }
            let _ = crate::typed::walk_expr_children(kind, &mut |child| {
                pending.push(child);
                ControlFlow::Continue(())
            });
        }
    }
    Some(writes)
}
