use ast::{ConstValue, Literal, Ty};
use std::collections::HashMap;

use crate::{ExprId, TypeRegistry, TypedExprKind, typed::TypedExprVec};

pub const PROMOTED_ARRAY_NODES_MAX: usize = 1_048_576;

pub fn promoted_array_value(
    expressions: &TypedExprVec,
    registry: &TypeRegistry,
    expression: ExprId,
) -> Option<ConstValue> {
    if !matches!(
        expressions.kind_of(expression)?,
        TypedExprKind::ArrayLit(_) | TypedExprKind::FixedArrayAlloc { .. }
    ) {
        return None;
    }
    let mut pending = vec![(expression, false)];
    let mut values = HashMap::new();
    let mut sizes = HashMap::new();
    let limit = expressions.kind.len().checked_mul(2)?;
    for _ in 0..=limit {
        let Some((id, children_ready)) = pending.pop() else {
            return values.remove(&expression);
        };
        if values.contains_key(&id) {
            continue;
        }
        let ty = expressions.ty_of(id)?;
        let definition = registry.resolved_definition_for_type(ty);
        if registry.integer_validity_for_type(ty).is_none()
            && !matches!(
                definition,
                Ty::AnonymousInteger { .. } | Ty::Float { .. } | Ty::Array { .. }
            )
        {
            return None;
        }
        let kind = expressions.kind_of(id)?;
        if !children_ready {
            pending.push((id, true));
            match kind {
                TypedExprKind::ArrayLit(items) => {
                    pending.extend(items.iter().rev().map(|id| (*id, false)))
                }
                TypedExprKind::FixedArrayAlloc { init, .. }
                | TypedExprKind::Cast { expr: init, .. } => pending.push((*init, false)),
                TypedExprKind::Lit(_) | TypedExprKind::Rematerialize { .. } => {}
                _ => return None,
            }
            continue;
        }
        let (value, size) = promoted_value(kind, registry, expressions, &values, &sizes)?;
        if size > PROMOTED_ARRAY_NODES_MAX {
            return None;
        }
        values.insert(id, value);
        sizes.insert(id, size);
    }
    None
}

fn promoted_value(
    kind: &TypedExprKind,
    registry: &TypeRegistry,
    expressions: &TypedExprVec,
    values: &HashMap<ExprId, ConstValue>,
    sizes: &HashMap<ExprId, usize>,
) -> Option<(ConstValue, usize)> {
    match kind {
        TypedExprKind::ArrayLit(items) => {
            let size = items
                .iter()
                .try_fold(1usize, |size, id| size.checked_add(*sizes.get(id)?))?;
            let value = items
                .iter()
                .map(|id| values.get(id).cloned())
                .collect::<Option<Vec<_>>>()?;
            Some((ConstValue::FixedArray(value.into()), size))
        }
        TypedExprKind::FixedArrayAlloc { init, size } => {
            let count = usize::try_from(size.as_const_int()?).ok()?;
            let size = sizes.get(init)?.checked_mul(count)?.checked_add(1)?;
            if size > PROMOTED_ARRAY_NODES_MAX {
                return None;
            }
            Some((
                ConstValue::FixedArray(vec![values.get(init)?.clone(); count].into()),
                size,
            ))
        }
        TypedExprKind::Cast { expr, ty } => {
            let value = values.get(expr)?.clone();
            let source_width = registry.integer_width_for_type(expressions.ty_of(*expr)?)?;
            let target_width = registry.integer_width_for_type(ty)?;
            let operation = if target_width < source_width {
                crate::intrinsic::IntrinsicOp::BitsTruncate
            } else if value.as_const_size_int().is_some_and(|value| value < 0) {
                crate::intrinsic::IntrinsicOp::BitsExtendSigned
            } else {
                crate::intrinsic::IntrinsicOp::BitsExtendUnsigned
            };
            let value = operation
                .fold(&[value], source_width, target_width)
                .ok()??;
            Some((value, *sizes.get(expr)?))
        }
        TypedExprKind::Rematerialize {
            constant: ConstValue::Int(value),
            ..
        }
        | TypedExprKind::Lit(Literal::Int(value)) => Some((ConstValue::Int(*value), 1)),
        TypedExprKind::Rematerialize {
            constant: ConstValue::Float(value),
            ..
        }
        | TypedExprKind::Lit(Literal::Float(value)) => Some((ConstValue::Float(*value), 1)),
        TypedExprKind::Lit(Literal::String(value)) => Some((
            ConstValue::FixedArray(
                value
                    .bytes()
                    .map(|byte| ConstValue::Int(byte.into()))
                    .collect::<Vec<_>>()
                    .into(),
            ),
            value.len().checked_add(1)?,
        )),
        _ => None,
    }
}
