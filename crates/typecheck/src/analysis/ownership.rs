use crate::ty::Ty;
use crate::typed::{DefId, ExprId, TypedCallableSignature, TypedExprKind, TypedFileAst};
use std::collections::HashMap;

pub trait TyOwnershipExt {
    fn is_structurally_discardable(&self) -> bool;
}

impl TyOwnershipExt for Ty {
    fn is_structurally_discardable(&self) -> bool {
        match self {
            Ty::AnonymousInteger { .. }
            | Ty::ResultFamily { .. }
            | Ty::UnresolvedLiteral(_)
            | Ty::Float { .. }
            | Ty::Unit
            | Ty::Ref { .. } => true,
            Ty::Tuple(items) => items
                .iter()
                .all(TyOwnershipExt::is_structurally_discardable),
            Ty::Record { fields, .. } => fields
                .iter()
                .all(|(_, field)| field.is_structurally_discardable()),
            Ty::Array { elem, .. } => elem.is_structurally_discardable(),
            Ty::Union { variants, .. } => variants.iter().all(|variant| {
                variant
                    .fields
                    .iter()
                    .all(|(_, field)| field.is_structurally_discardable())
            }),
            Ty::Named { .. }
            | Ty::Opaque(_)
            | Ty::Ptr { .. }
            | Ty::Address { .. }
            | Ty::Literal(_) => false,
        }
    }
}

pub fn expr_is_rematerializable(typed: &TypedFileAst, expr: ExprId) -> bool {
    let Some(ty) = typed.exprs.ty_of(expr) else {
        return false;
    };
    if ty.named_instance_stripping_reference_wrappers().is_some() {
        return false;
    }
    match ty {
        Ty::AnonymousInteger { .. } => typed
            .exprs
            .integer_knowledge_of(expr)
            .and_then(Option::as_ref)
            .is_some_and(|knowledge| {
                matches!(
                    knowledge,
                    ast::integer::IntegerKnowledge::Exact(
                        ast::integer::CanonicalIntegerExpr::Value(_)
                    )
                )
            }),
        Ty::ResultFamily { .. } => matches!(
            typed.exprs.const_value_of(expr),
            Some(Some(ast::ConstValue::ResultAlternative { .. }))
        ),
        _ => false,
    }
}

pub(crate) fn stage_rematerialize_owned_arguments(
    typed: &mut TypedFileAst,
    cross_file_signatures: &HashMap<DefId, TypedCallableSignature>,
) {
    propagate_condition_singletons(typed);
    let original_len = typed.exprs.kind.len();
    for index in 0..original_len {
        if let TypedExprKind::Cast { expr, .. } = typed.exprs.kind[index] {
            propagate_local_constant_to_reference(typed, expr);
            if expr_is_rematerializable(typed, expr) {
                let rematerialized = append_rematerialization(typed, expr);
                let TypedExprKind::Cast { expr, .. } = &mut typed.exprs.kind[index] else {
                    unreachable!()
                };
                *expr = rematerialized;
            }
            continue;
        }
        let TypedExprKind::FnCall {
            target,
            args: Some(arguments),
            ..
        } = &typed.exprs.kind[index]
        else {
            continue;
        };
        let target = *target;
        let arguments = arguments.clone();
        let conventions = typed
            .defs
            .get(&target)
            .map(|definition| definition.param_conventions.clone())
            .or_else(|| {
                cross_file_signatures
                    .get(&target)
                    .map(|signature| signature.param_conventions.clone())
            })
            .unwrap_or_default();
        for argument in &arguments {
            propagate_local_constant_to_reference(typed, *argument);
        }
        let rewritten: Vec<_> = arguments
            .into_iter()
            .enumerate()
            .map(|(argument_index, argument)| {
                if conventions.get(argument_index) == Some(&ast::ParamConvention::Own)
                    && expr_is_rematerializable(typed, argument)
                {
                    append_rematerialization(typed, argument)
                } else {
                    argument
                }
            })
            .collect();
        let TypedExprKind::FnCall { args, .. } = &mut typed.exprs.kind[index] else {
            unreachable!()
        };
        *args = Some(rewritten);
    }
}

fn propagate_local_constant_to_reference(typed: &mut TypedFileAst, reference: ExprId) {
    let TypedExprKind::FnCall {
        target, args: None, ..
    } = typed.exprs.kind[reference.as_usize()]
    else {
        return;
    };
    if !matches!(
        typed.exprs.ty[reference.as_usize()],
        Ty::AnonymousInteger { .. }
    ) {
        return;
    }
    let source = typed.exprs.kind[..reference.as_usize()]
        .iter()
        .rev()
        .find_map(|kind| match kind {
            TypedExprKind::Bind { name, body, .. } if *name == target.0 => Some(*body),
            TypedExprKind::Reassign { name, value, .. } if *name == target.0 => Some(*value),
            _ => None,
        });
    let constant = source.and_then(|source| closed_constant(typed, source));
    let Some(ast::ConstValue::Int(value)) = constant else {
        return;
    };
    typed.exprs.const_value[reference.as_usize()] = Some(ast::ConstValue::Int(value));
    typed.exprs.integer_knowledge[reference.as_usize()] = Some(
        ast::integer::IntegerKnowledge::Exact(ast::integer::CanonicalIntegerExpr::Value(value)),
    );
}

fn closed_constant(typed: &TypedFileAst, expression: ExprId) -> Option<ast::ConstValue> {
    if let Some(value) = typed.exprs.const_value[expression.as_usize()].clone() {
        return Some(value);
    }
    let TypedExprKind::When(when_expr) = &typed.exprs.kind[expression.as_usize()] else {
        return None;
    };
    if let Some(subject) = when_expr.subject
        && let Some(subject_value) = typed.exprs.const_value[subject.as_usize()].as_ref()
    {
        for arm in &when_expr.arms {
            match arm {
                crate::typed::TypedWhenArm::Is { pattern, body, .. }
                    if crate::analysis::pattern::pattern_matches_with_tag_types(
                        &pattern.value,
                        subject_value,
                        &typed.tag_types,
                    ) =>
                {
                    return closed_constant(typed, *body);
                }
                crate::typed::TypedWhenArm::Else(body, _) => {
                    return closed_constant(typed, *body);
                }
                _ => {}
            }
        }
    }
    let mut values = when_expr.arms.iter().map(|arm| match arm {
        crate::typed::TypedWhenArm::Cond { body, .. }
        | crate::typed::TypedWhenArm::Is { body, .. }
        | crate::typed::TypedWhenArm::Else(body, _) => closed_constant(typed, *body),
    });
    let first = values.next()??;
    values
        .all(|value| value.as_ref() == Some(&first))
        .then_some(first)
}

fn propagate_condition_singletons(typed: &mut TypedFileAst) {
    let conditions: Vec<_> = typed
        .exprs
        .kind
        .iter()
        .filter_map(|kind| {
            let TypedExprKind::If(if_expr) = kind else {
                return None;
            };
            let crate::typed::TypedCondition::Is { subject, pattern } = &if_expr.condition else {
                return None;
            };
            let TypedExprKind::FnCall {
                target, args: None, ..
            } = typed.exprs.kind[subject.as_usize()].clone()
            else {
                return None;
            };
            let ast::Pattern::Literal(ast::Literal::Int(value), _) = pattern.value else {
                return None;
            };
            Some((
                target.0,
                value,
                if_expr
                    .stmts
                    .iter()
                    .copied()
                    .chain(if_expr.ret)
                    .collect::<Vec<_>>(),
            ))
        })
        .collect();
    for (name, value, roots) in conditions {
        let mut references = Vec::new();
        for root in roots {
            let _ = crate::typed::walk_expr_preorder(typed, root, &mut |expression| {
                if matches!(
                    &typed.exprs.kind[expression.as_usize()],
                    TypedExprKind::FnCall { target, args: None, .. } if target.0 == name
                ) {
                    references.push(expression);
                }
                std::ops::ControlFlow::Continue(())
            });
        }
        for reference in references {
            typed.exprs.const_value[reference.as_usize()] = Some(ast::ConstValue::Int(value));
            typed.exprs.integer_knowledge[reference.as_usize()] =
                Some(ast::integer::IntegerKnowledge::Exact(
                    ast::integer::CanonicalIntegerExpr::Value(value),
                ));
        }
    }
}

fn append_rematerialization(typed: &mut TypedFileAst, source: ExprId) -> ExprId {
    let constant = typed.exprs.const_value[source.as_usize()]
        .clone()
        .or_else(|| {
            let ast::integer::IntegerKnowledge::Exact(ast::integer::CanonicalIntegerExpr::Value(
                value,
            )) = typed.exprs.integer_knowledge[source.as_usize()].as_ref()?
            else {
                return None;
            };
            Some(ast::ConstValue::Int(*value))
        })
        .expect("rematerializable expressions carry a closed constant proof");
    let id = ExprId(typed.exprs.kind.len() as u32);
    typed.exprs.kind.push(TypedExprKind::Rematerialize {
        source,
        constant: constant.clone(),
    });
    typed
        .exprs
        .ty
        .push(typed.exprs.ty[source.as_usize()].clone());
    typed.exprs.span.push(typed.exprs.span[source.as_usize()]);
    typed.exprs.const_value.push(Some(constant));
    typed
        .exprs
        .integer_knowledge
        .push(typed.exprs.integer_knowledge[source.as_usize()].clone());
    typed
        .exprs
        .result_evidence
        .push(typed.exprs.result_evidence[source.as_usize()].clone());
    typed
        .exprs
        .projected_result_evidence
        .push(typed.exprs.projected_result_evidence[source.as_usize()].clone());
    typed
        .exprs
        .availability
        .push(crate::Availability::CompileTime);
    typed.exprs.target_group.push(None);
    typed.exprs.place.push(None);
    typed.exprs.place_version.push(None);
    typed.exprs.flaws.push(Vec::new());
    id
}
