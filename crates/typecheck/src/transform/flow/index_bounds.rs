use super::*;
use crate::transform::symbolic_values::field_symbol;
use crate::typed::{BindBody, SequenceIndexCheck, TypedBind};

fn parameter_projection(
    typed: &TypedFileAst,
    bind: &TypedBind,
    expression: ExprId,
) -> Option<NormalExpr> {
    match typed.exprs.kind_of(expression)? {
        TypedExprKind::FnCall {
            target, args: None, ..
        } if bind.params.iter().any(|(name, _)| *name == target.0) => {
            Some(NormalExpr::Var(target.0))
        }
        TypedExprKind::AggregateGet { base, index } => {
            let NormalExpr::Var(root) = parameter_projection(typed, bind, *base)? else {
                return None;
            };
            Some(field_symbol(root, *index))
        }
        TypedExprKind::Deref(inner)
        | TypedExprKind::Ref(inner)
        | TypedExprKind::Rematerialize { source: inner, .. } => {
            parameter_projection(typed, bind, *inner)
        }
        _ => None,
    }
}

pub(super) fn parameter_index_check(
    typed: &TypedFileAst,
    bind: &TypedBind,
    constraints: &ConstraintEnv,
) -> Option<(ExprId, SequenceIndexCheck)> {
    let expression = match &bind.body {
        BindBody::Expr(expression) => *expression,
        BindBody::Body {
            exprs,
            ret: Some(expression),
        } if exprs.is_empty() => *expression,
        _ => return None,
    };
    let expression = match typed.exprs.kind_of(expression)? {
        TypedExprKind::Ref(inner) => *inner,
        _ => expression,
    };
    let TypedExprKind::BufGet { buf, index } = typed.exprs.kind_of(expression)? else {
        return None;
    };
    let proven = || {
        let ty = match typed.exprs.kind_of(*buf)? {
            TypedExprKind::AggregateGet { base, index } => {
                super::super::symbolic_values::projected_type(typed, *base, *index)?
            }
            _ => typed.exprs.ty_of(*buf)?.clone(),
        };
        let Ty::Array { size: length, .. } = typed
            .type_registry
            .resolved_definition_for_type(ty.without_reference_wrappers())
        else {
            return None;
        };
        let position = parameter_projection(typed, bind, *index)?;
        let mut constraints = constraints.clone();
        super::record_constraints::assume_integer_bounds(
            typed,
            typed.exprs.ty_of(*index)?,
            position.clone(),
            &mut constraints,
        );
        let values = HashMap::new();
        (constraints.prove(&Predicate::Ge(position.clone(), 0_i128.into()), &values)
            == ProveResult::Proven
            && constraints.prove(&Predicate::Lt(position, length), &values) == ProveResult::Proven)
            .then_some(SequenceIndexCheck::Proven {
                buffer: *buf,
                index: *index,
            })
    };
    Some((expression, proven().unwrap_or(SequenceIndexCheck::Runtime)))
}
