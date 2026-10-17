use std::collections::{HashMap, HashSet};

use ast::{ConstValue, NormalExpr, Ty};
use internment::Intern;

use super::{callable_signatures::CallableSignatureScope, flow::expr_to_normal_expr};
use crate::typed::{ExprId, TypedExprKind, TypedFileAst};

pub(super) fn field_symbol(base: Intern<String>, index: usize) -> NormalExpr {
    NormalExpr::Var(Intern::new(format!("{base}.$field{index}")))
}

pub(super) fn field_path_symbol(mut root: Intern<String>, fields: &[usize]) -> NormalExpr {
    for field in fields {
        let NormalExpr::Var(projected) = field_symbol(root, *field) else {
            unreachable!()
        };
        root = projected;
    }
    NormalExpr::Var(root)
}

pub(super) fn payload_symbol(base: Intern<String>, variant: usize) -> Intern<String> {
    Intern::new(format!("{base}.$variant{variant}"))
}

pub(super) fn call_result_symbol(expression: ExprId) -> NormalExpr {
    NormalExpr::Var(Intern::new(format!("$call{}", expression.0)))
}

pub(super) fn address_type(typed: &TypedFileAst, ty: &Ty) -> bool {
    matches!(
        typed.type_registry.resolved_definition_for_type(ty),
        Ty::Ptr { .. } | Ty::OwnedStorage { .. }
    )
}

pub(super) fn scalar_type(typed: &TypedFileAst, ty: &Ty) -> bool {
    typed.type_registry.integer_validity_for_type(ty).is_some() || address_type(typed, ty)
}

fn preserves_address_cast(typed: &TypedFileAst, source: &Ty, target: &Ty) -> bool {
    let source_address = address_type(typed, source.without_reference_wrappers());
    let target_address = address_type(typed, target.without_reference_wrappers());
    if source_address && target_address {
        return true;
    }
    if !source_address && !target_address {
        return false;
    }
    let integer = if source_address { target } else { source };
    let Some(layout) = typed.target_layout else {
        return pointer_width_integer(typed, integer);
    };
    let maximum = (i256::I256::from(1) << layout.pointer_bits() as u32) - i256::I256::from(1);
    let Some(hull) = typed
        .type_registry
        .integer_validity_for_type(integer.without_reference_wrappers())
        .and_then(|validity| validity.domain().storage_hull())
    else {
        return false;
    };
    if source_address {
        hull.min() == 0.into() && hull.max() >= maximum
    } else {
        target_address && hull.min() >= 0.into() && hull.max() <= maximum
    }
}

fn pointer_width_integer(typed: &TypedFileAst, ty: &Ty) -> bool {
    use ast::integer::{CanonicalNaturalExpr, IntegerInterpretation, IntegerRepresentationRule};
    let Some(integer) = typed
        .type_registry
        .nominal_integer_semantics_for_type(ty.without_reference_wrappers())
    else {
        return false;
    };
    if integer.interpretation != IntegerInterpretation::Unsigned {
        return false;
    }
    let IntegerRepresentationRule::ExactBits(CanonicalNaturalExpr::Symbolic(expression)) =
        &integer.representation_rule
    else {
        return false;
    };
    let mut expression = expression.clone();
    for _ in 0..16 {
        match expression {
            NormalExpr::TargetQuery {
                kind: ast::TargetQueryKind::IndexBits,
                operand,
            } => {
                return matches!(operand, ast::TypeReference::Pointer(_));
            }
            NormalExpr::Var(name) => {
                let Some(value) = symbolic_constant(typed, name) else {
                    return false;
                };
                expression = value;
            }
            _ => return false,
        }
    }
    false
}

fn symbolic_constant(typed: &TypedFileAst, name: Intern<String>) -> Option<NormalExpr> {
    super::semantic_reads::body(crate::typed::DefId::named(name));
    if let Some(bind) = typed.eval_ast.defs.get(&name) {
        if bind.params.is_some() {
            return None;
        }
        let ast::BindValue::Expr(expression) = &bind.value else {
            return None;
        };
        return match &expression.value {
            ast::Expr::TargetQuery { kind, operand } => Some(NormalExpr::TargetQuery {
                kind: *kind,
                operand: operand.clone(),
            }),
            ast::Expr::FnCall(call) if call.args.is_none() => Some(NormalExpr::Var(Intern::new(
                call.path.value.to_qualified_string(),
            ))),
            _ => None,
        };
    }
    let id = crate::typed::DefId::named(name);
    let bind = typed.defs.get(&id)?;
    super::semantic_reads::body(id);
    let crate::typed::BindBody::Expr(body) = &bind.body else {
        return None;
    };
    normal_expr(typed, None, &HashMap::new(), *body)
}

pub(super) fn projected_type(typed: &TypedFileAst, base: ExprId, index: usize) -> Option<Ty> {
    let ty = typed.exprs.ty_of(base)?.without_reference_wrappers();
    match typed.type_registry.resolved_definition_for_type(ty) {
        Ty::Record { fields, .. } => {
            let root = normal_expr(typed, None, &HashMap::new(), base)
                .filter(|value| matches!(value, NormalExpr::Var(_)))
                .unwrap_or_else(|| call_result_symbol(base));
            let NormalExpr::Var(root) = root else {
                unreachable!();
            };
            let mut substitution = crate::subst::DepSubst::new();
            for (index, (name, _)) in fields.iter().enumerate() {
                substitution.consts.insert(*name, field_symbol(root, index));
            }
            let mut field = substitution.apply_to_ty(&fields.get(index)?.1);
            if typed.exprs.ty_of(base)?.is_ref()
                && !typed.exprs.ty_of(base)?.is_mutable_ref()
                && let Ty::Ref { mutable, .. } = &mut field
            {
                *mutable = false;
            }
            Some(field)
        }
        Ty::Sequence(fields) => fields.get(index).map(|field| *field.ty.clone()),
        _ => None,
    }
}

pub(super) fn normal_expr(
    typed: &TypedFileAst,
    signatures: Option<&CallableSignatureScope<'_>>,
    values: &HashMap<Intern<String>, ConstValue>,
    expression: ExprId,
) -> Option<NormalExpr> {
    fn mutates(
        typed: &TypedFileAst,
        signatures: Option<&CallableSignatureScope<'_>>,
        expression: ExprId,
    ) -> bool {
        let mut pending = vec![expression];
        let mut visited = HashSet::new();
        while let Some(expression) = pending.pop() {
            if !visited.insert(expression) {
                continue;
            }
            match typed.exprs.kind_of(expression) {
                Some(
                    TypedExprKind::Reassign { .. }
                    | TypedExprKind::RecordSet { .. }
                    | TypedExprKind::AggregateSet { .. }
                    | TypedExprKind::BufSet { .. }
                    | TypedExprKind::Emit { .. },
                ) => return true,
                Some(TypedExprKind::FnCall { target, .. })
                    if typed.exprs.place_of(expression).flatten().is_none()
                        && signatures
                            .and_then(|signatures| signatures.get(target))
                            .is_some_and(|signature| {
                                signature.effects.unknown_memory
                                    || !signature.effects.writes.is_empty()
                                    || signature
                                        .param_conventions
                                        .contains(&ast::ParamConvention::Mutate)
                            }) =>
                {
                    return true;
                }
                _ => {}
            }
            let _ = crate::typed::walk_expr_children_of(typed, expression, &mut |child| {
                pending.push(child);
                std::ops::ControlFlow::Continue(())
            });
        }
        false
    }

    fn resolve(
        typed: &TypedFileAst,
        signatures: Option<&CallableSignatureScope<'_>>,
        values: &HashMap<Intern<String>, ConstValue>,
        expression: ExprId,
        depth: usize,
    ) -> Option<NormalExpr> {
        if depth == 32 {
            return None;
        }
        let recurse = |expression| resolve(typed, signatures, values, expression, depth + 1);
        match typed.exprs.kind_of(expression)? {
            TypedExprKind::TagCall {
                args: Some(args),
                field_names,
                ..
            } => {
                let Ty::Record { fields, .. } = typed
                    .type_registry
                    .resolved_definition_for_type(typed.exprs.ty_of(expression)?)
                else {
                    return expr_to_normal_expr(
                        &typed.exprs.kind,
                        &typed.exprs.facts.const_facts.const_value,
                        values,
                        expression,
                    );
                };
                let mut ordered = vec![None; fields.len()];
                for (slot, argument) in args.iter().enumerate() {
                    let index = field_names
                        .get(slot)
                        .and_then(|name| fields.iter().position(|(field, _)| field == name))
                        .unwrap_or(slot);
                    let NormalExpr::Value(value) = recurse(*argument)? else {
                        return None;
                    };
                    *ordered.get_mut(index)? = Some((fields.get(index)?.0, value));
                }
                Some(NormalExpr::Value(ConstValue::Record {
                    fields: ordered.into_iter().collect::<Option<Vec<_>>>()?.into(),
                }))
            }
            TypedExprKind::Bind {
                body,
                stmts,
                unassigned: false,
                ..
            } if stmts.is_empty() => recurse(*body),
            TypedExprKind::Ref(inner) | TypedExprKind::Rematerialize { source: inner, .. } => {
                recurse(*inner)
            }
            TypedExprKind::TakePtr(_) => None,
            TypedExprKind::Deref(inner) => typed
                .exprs
                .ty_of(*inner)
                .is_some_and(Ty::is_ref)
                .then(|| recurse(*inner))
                .flatten(),
            TypedExprKind::Cast { expr, .. } => {
                let source = &typed.exprs.ty[expr.index()];
                let target = &typed.exprs.ty[expression.index()];
                if preserves_address_cast(typed, source, target)
                    || typed
                        .type_registry
                        .integer_validity_for_type(source.without_reference_wrappers())
                        .is_some()
                        && typed
                            .type_registry
                            .integer_validity_for_type(target.without_reference_wrappers())
                            .is_some()
                {
                    recurse(*expr)
                } else {
                    None
                }
            }
            TypedExprKind::UnionPayload { expr, variant } => {
                let NormalExpr::Var(base) = recurse(*expr)? else {
                    return None;
                };
                Some(NormalExpr::Var(payload_symbol(base, *variant)))
            }
            TypedExprKind::AggregateGet { base, index } => match recurse(*base)? {
                NormalExpr::Var(base) => Some(field_symbol(base, *index)),
                NormalExpr::Value(ConstValue::Record { fields }) => fields
                    .get(*index)
                    .map(|(_, value)| NormalExpr::Value(value.clone())),
                _ => None,
            },
            TypedExprKind::When(_)
                if typed
                    .type_registry
                    .integer_validity_for_type(typed.exprs.ty_of(expression)?)
                    .is_some()
                    || matches!(
                        typed
                            .type_registry
                            .resolved_definition_for_type(typed.exprs.ty_of(expression)?),
                        Ty::Record { .. } | Ty::Union { .. }
                    ) =>
            {
                Some(call_result_symbol(expression))
            }
            TypedExprKind::FnCall { target, args, .. }
                if args.as_ref().is_none_or(Vec::is_empty)
                    && typed.exprs.place_of(expression).flatten().is_none()
                    && typed
                        .eval_ast
                        .defs
                        .get(target)
                        .is_some_and(|bind| bind.params.is_none() && bind.is_constant()) =>
            {
                Some(
                    typed
                        .exprs
                        .const_value_of(expression)
                        .and_then(Option::as_ref)
                        .cloned()
                        .map(NormalExpr::Value)
                        .unwrap_or(NormalExpr::Var(target.0)),
                )
            }
            TypedExprKind::FnCall {
                target, args: None, ..
            } if typed.exprs.place_of(expression).flatten().is_some() => Some(
                values
                    .get(&target.0)
                    .filter(|value| {
                        !matches!(value, ConstValue::Tag { .. } | ConstValue::Record { .. })
                    })
                    .cloned()
                    .map(NormalExpr::Value)
                    .unwrap_or(NormalExpr::Var(target.0)),
            ),
            TypedExprKind::FnCall { .. }
                if matches!(
                    typed
                        .type_registry
                        .resolved_definition_for_type(typed.exprs.ty_of(expression)?),
                    Ty::Record { .. } | Ty::Union { .. }
                ) =>
            {
                Some(call_result_symbol(expression))
            }
            TypedExprKind::FnCall { target, args, .. }
                if signatures
                    .and_then(|scope| scope.get(target))
                    .is_some_and(|signature| signature.effects.scalar_result.is_some())
                    || args.as_ref().is_some_and(|args| !args.is_empty()) =>
            {
                let signature = signatures.and_then(|scope| scope.get(target));
                if args
                    .iter()
                    .flatten()
                    .any(|arg| mutates(typed, signatures, *arg))
                {
                    return None;
                }
                let bind = typed.defs.get(target);
                let effects = signature.map(|signature| &signature.effects).or_else(|| {
                    super::semantic_reads::body(*target);
                    bind.map(|bind| &bind.effects)
                })?;
                if !effects.writes.is_empty() || effects.unknown_memory {
                    return None;
                }
                let result = effects.scalar_result.clone().or_else(|| {
                    super::semantic_reads::body(*target);
                    let body = match &bind?.body {
                        crate::typed::BindBody::Expr(body) => *body,
                        crate::typed::BindBody::Body {
                            exprs,
                            ret: Some(body),
                        } if exprs.is_empty() => *body,
                        _ => return None,
                    };
                    recurse(body)
                });
                let Some(result) = result else {
                    return scalar_type(typed, typed.exprs.ty_of(expression)?)
                        .then(|| call_result_symbol(expression));
                };
                let params = signature
                    .map(|signature| signature.params.as_slice())
                    .or_else(|| {
                        super::semantic_reads::body(*target);
                        bind.map(|bind| &bind.params[..])
                    })?;
                let mut substitution = crate::subst::DepSubst::new();
                let mut names = HashSet::new();
                super::scalar_mutations::variables(&result, &mut names);
                fn mentions(expression: &NormalExpr, name: Intern<String>) -> bool {
                    match expression {
                        NormalExpr::Var(value) => {
                            *value == name || value.as_str().starts_with(&format!("{name}.$field"))
                        }
                        NormalExpr::Add(left, right)
                        | NormalExpr::Sub(left, right)
                        | NormalExpr::Mul(left, right) => {
                            mentions(left, name) || mentions(right, name)
                        }
                        _ => false,
                    }
                }
                for ((name, _), arg) in params.iter().zip(args.iter().flatten()) {
                    if let Some(value) = recurse(*arg) {
                        for projection in &names {
                            if let Some(suffix) =
                                projection.as_str().strip_prefix(&format!("{name}.$field"))
                            {
                                let projected = match &value {
                                    NormalExpr::Var(base) => NormalExpr::Var(Intern::new(format!(
                                        "{base}.$field{suffix}"
                                    ))),
                                    NormalExpr::Value(ConstValue::Record { fields }) => {
                                        let index: usize = suffix.parse().ok()?;
                                        NormalExpr::Value(fields.get(index)?.1.clone())
                                    }
                                    _ => return None,
                                };
                                substitution.consts.insert(*projection, projected);
                            }
                        }
                        substitution.consts.insert(*name, value);
                    } else if mentions(&result, *name) {
                        return None;
                    }
                }
                Some(substitution.apply_to_normal(&result))
            }
            TypedExprKind::IntrinsicCall { op, args } if args.len() == 2 => {
                let left = Box::new(recurse(args[0])?);
                let right = Box::new(recurse(args[1])?);
                match op {
                    crate::typed::IntrinsicOp::BitsAdd => Some(NormalExpr::Add(left, right)),
                    crate::typed::IntrinsicOp::BitsSubtract => Some(NormalExpr::Sub(left, right)),
                    crate::typed::IntrinsicOp::BitsMultiply => Some(NormalExpr::Mul(left, right)),
                    _ => None,
                }
            }
            _ => expr_to_normal_expr(
                &typed.exprs.kind,
                &typed.exprs.facts.const_facts.const_value,
                values,
                expression,
            ),
        }
    }
    resolve(typed, signatures, values, expression, 0)
}

#[cfg(test)]
#[path = "symbolic_values_tests.rs"]
mod tests;
