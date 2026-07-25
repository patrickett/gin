use ast::ParamConvention;
use diagnostic::Diagnostic;
use internment::Intern;

use crate::ty::Ty;
use crate::typed::{BindBody, ExprId, ReferenceTargetGroup, TypedExprKind, TypedFileAst};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArgumentMode {
    Own,
    Observe,
    Mutate,
    Consume,
}

fn argument_mode(typed: &TypedFileAst, expr_id: ExprId) -> ArgumentMode {
    let idx = expr_id.as_usize();
    match typed.exprs.kind.get(idx) {
        Some(TypedExprKind::ConsumeArg(_)) => ArgumentMode::Consume,
        Some(TypedExprKind::Ref(_)) => match typed.exprs.ty.get(idx) {
            Some(Ty::Ref { mutable: true, .. }) => ArgumentMode::Mutate,
            _ => ArgumentMode::Observe,
        },
        _ => ArgumentMode::Own,
    }
}

fn is_addressable_place(typed: &TypedFileAst, expr_id: ExprId) -> bool {
    match typed.exprs.kind.get(expr_id.as_usize()) {
        Some(TypedExprKind::FnCall { args, .. }) => {
            args.as_ref().is_none_or(|args| args.is_empty())
        }
        Some(TypedExprKind::TupleGet { base, .. }) => is_addressable_place(typed, *base),
        Some(TypedExprKind::BufGet { buf, .. }) => is_addressable_place(typed, *buf),
        Some(TypedExprKind::Deref(_)) => true,
        _ => false,
    }
}

fn expected_mode(convention: ParamConvention) -> ArgumentMode {
    match convention {
        ParamConvention::Own => ArgumentMode::Own,
        ParamConvention::Observe => ArgumentMode::Observe,
        ParamConvention::Mutate => ArgumentMode::Mutate,
        ParamConvention::Consume => ArgumentMode::Consume,
    }
}

fn mode_name(mode: ArgumentMode) -> &'static str {
    match mode {
        ArgumentMode::Own => "owned",
        ArgumentMode::Observe => "ref",
        ArgumentMode::Mutate => "mut",
        ArgumentMode::Consume => "eat",
    }
}

fn emit_return_consumed_param_flaws(typed: &mut TypedFileAst) {
    let bodies: Vec<(Vec<Intern<String>>, Vec<ExprId>)> = typed
        .defs
        .values()
        .filter_map(|bind| {
            let consumed: Vec<_> = bind
                .params
                .iter()
                .zip(&bind.param_conventions)
                .filter_map(|((name, _), convention)| {
                    (*convention == ParamConvention::Consume).then_some(*name)
                })
                .collect();
            if consumed.is_empty() {
                return None;
            }
            let roots = match &bind.body {
                BindBody::Expr(id) => vec![*id],
                BindBody::Body { ret: Some(id), .. } => vec![*id],
                BindBody::Body { ret: None, .. } | BindBody::Extern => vec![],
            };
            Some((consumed, roots))
        })
        .collect();

    for (consumed, roots) in bodies {
        for root in roots {
            emit_return_consumed_in_expr(typed, root, &consumed);
        }
    }
}

fn emit_return_consumed_in_expr(
    typed: &mut TypedFileAst,
    expr_id: ExprId,
    consumed: &[Intern<String>],
) {
    let idx = expr_id.as_usize();
    if idx >= typed.exprs.kind.len() {
        return;
    }
    if let TypedExprKind::FnCall { target, args, .. } = &typed.exprs.kind[idx]
        && args.as_ref().is_none_or(|a| a.is_empty())
        && consumed.contains(&target.0)
    {
        typed.exprs.flaws[idx].push(Diagnostic::new(
            "type-return-consumed-param",
            format!("cannot return consumed parameter `{}`", target.0.as_str()),
        ));
    }
}

fn validate_reference_returns(typed: &mut TypedFileAst) {
    let returns: Vec<_> = typed
        .defs
        .values()
        .filter_map(|bind| {
            let expected = bind.return_group?;
            let expr = match &bind.body {
                BindBody::Expr(expr) => Some(*expr),
                BindBody::Body { ret, .. } => *ret,
                BindBody::Extern => None,
            }?;
            Some((expr, expected))
        })
        .collect();

    for (expr, expected) in returns {
        let actual = typed
            .exprs
            .target_group
            .get(expr.as_usize())
            .cloned()
            .flatten();
        let (code, message) = match actual {
            Some(ReferenceTargetGroup::Param(group)) if group == expected => continue,
            Some(ReferenceTargetGroup::Param(_)) => (
                "type-reference-return-wrong-target-group",
                "returned reference comes from the wrong target group",
            ),
            Some(
                ReferenceTargetGroup::Local(_)
                | ReferenceTargetGroup::Field { .. }
                | ReferenceTargetGroup::ItemRegion { .. }
                | ReferenceTargetGroup::ItemRange { .. }
                | ReferenceTargetGroup::Deref(_),
            ) => (
                "type-reference-return-local-target",
                "cannot return a reference to local storage",
            ),
            None => (
                "type-reference-return-unknown-target",
                "cannot determine the returned reference target",
            ),
        };
        typed.exprs.flaws[expr.as_usize()].push(Diagnostic::new(code, message));
    }
}

pub fn stage_validate_consumption(typed: &mut TypedFileAst) {
    validate_reference_returns(typed);
    emit_return_consumed_param_flaws(typed);

    let callable_defs: Vec<_> = typed
        .defs
        .iter()
        .map(|(def_id, bind)| (*def_id, bind.params.clone(), bind.param_conventions.clone()))
        .collect();

    for (target_def, params, conventions) in callable_defs {
        for idx in 0..typed.exprs.kind.len() {
            let TypedExprKind::FnCall { target, args, .. } = &typed.exprs.kind[idx] else {
                continue;
            };
            if *target != target_def {
                continue;
            }
            let Some(args) = args else {
                continue;
            };
            for (param_idx, ((param_name, _), convention)) in
                params.iter().zip(&conventions).enumerate()
            {
                let Some(arg_id) = args.get(param_idx) else {
                    continue;
                };
                let actual = argument_mode(typed, *arg_id);
                let expected = expected_mode(*convention);
                if actual == expected {
                    if matches!(actual, ArgumentMode::Observe | ArgumentMode::Mutate)
                        && let Some(TypedExprKind::Ref(inner)) =
                            typed.exprs.kind.get(arg_id.as_usize())
                        && !is_addressable_place(typed, *inner)
                    {
                        typed.exprs.flaws[idx].push(
                            Diagnostic::new(
                                "type-reference-to-temporary",
                                format!(
                                    "{} argument for parameter `{}` must be an addressable place",
                                    mode_name(actual),
                                    param_name
                                ),
                            )
                            .with_arg("name", param_name.as_str().to_string())
                            .with_arg("mode", mode_name(actual)),
                        );
                    }
                    continue;
                }
                let code = match (expected, actual) {
                    (ArgumentMode::Consume, _) => "type-consume-required",
                    (_, ArgumentMode::Consume) => "type-consume-arg-on-bare-param",
                    _ => "type-argument-mode-mismatch",
                };
                typed.exprs.flaws[idx].push(
                    Diagnostic::new(
                        code,
                        format!(
                            "parameter `{}` expects {}, found {}",
                            param_name,
                            mode_name(expected),
                            mode_name(actual)
                        ),
                    )
                    .with_arg("name", param_name.as_str().to_string())
                    .with_arg("expected", mode_name(expected))
                    .with_arg("actual", mode_name(actual)),
                );
            }
        }
    }
}
