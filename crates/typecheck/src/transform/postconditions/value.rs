use super::*;
use ast::ParamConvention;
use internment::Intern;

pub(in crate::transform) fn stable(
    typed: &TypedFileAst,
    bind: &TypedBind,
    name: Intern<String>,
    signatures: &super::super::callable_signatures::CallableSignatureScope<'_>,
) -> bool {
    if !bind
        .params
        .iter()
        .zip(&bind.param_conventions)
        .any(|((parameter, ty), convention)| {
            *parameter == name
                && !matches!(
                    convention,
                    ParamConvention::Observe | ParamConvention::Mutate
                )
                && typed.type_registry.integer_validity_for_type(ty).is_some()
        })
    {
        return false;
    }
    fn mentions(typed: &TypedFileAst, expression: ExprId, name: Intern<String>) -> bool {
        let mut found = false;
        let _ = crate::typed::walk_expr_preorder(typed, expression, &mut |expression| {
            found |= matches!(typed.exprs.kind_of(expression), Some(TypedExprKind::FnCall { target, args: None, .. }) if target.0 == name);
            if found {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        });
        found
    }
    let roots: Vec<_> = match &bind.body {
        BindBody::Expr(root) => vec![*root],
        BindBody::Body { exprs, ret } => exprs.iter().copied().chain(*ret).collect(),
        BindBody::Unlowered => return false,
    };
    let mut stable = true;
    for root in roots {
        let _ = crate::typed::walk_expr_preorder(typed, root, &mut |expression| {
            match typed.exprs.kind_of(expression) {
                Some(
                    TypedExprKind::Bind { name: other, .. }
                    | TypedExprKind::Reassign { name: other, .. },
                ) if *other == name => stable = false,
                Some(TypedExprKind::TakePtr(inner)) if mentions(typed, *inner, name) => {
                    stable = false
                }
                Some(TypedExprKind::FnCall {
                    target,
                    args: Some(args),
                    ..
                }) => {
                    for (slot, _argument) in args
                        .iter()
                        .enumerate()
                        .filter(|(_, argument)| mentions(typed, **argument, name))
                    {
                        let Some(signature) = signatures.get(target) else {
                            stable = false;
                            continue;
                        };
                        if matches!(
                            signature.param_conventions.get(slot),
                            Some(ParamConvention::Observe | ParamConvention::Mutate)
                        ) {
                            let harmless_result = signature.return_type == ast::Ty::Unit
                                || (typed
                                    .type_registry
                                    .integer_validity_for_type(&signature.return_type)
                                    .is_some()
                                    && signature.effects.scalar_result.is_some())
                                || matches!(typed.type_registry.resolved_definition_for_type(&signature.return_type), ast::Ty::Union { variants, .. } if variants.iter().all(|variant| variant.fields.is_empty()));
                            stable &= signature.param_conventions.get(slot)
                                == Some(&ParamConvention::Observe)
                                && !signature.effects.unknown_memory
                                && signature.effects.writes.is_empty()
                                && harmless_result;
                        }
                    }
                }
                _ => {}
            }
            if stable {
                std::ops::ControlFlow::Continue(())
            } else {
                std::ops::ControlFlow::Break(())
            }
        });
    }
    stable
}
