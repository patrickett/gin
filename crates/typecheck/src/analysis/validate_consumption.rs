use ast::ParamConvention;
use diagnostic::{Diagnostic, RelatedSpan};

use crate::typed::{BindBody, ExprId, TypedExprKind, TypedFileAst};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArgumentMode {
    Own,
    Observe,
    Mutate,
}

fn argument_mode(typed: &TypedFileAst, expr_id: ExprId) -> ArgumentMode {
    match typed.exprs.kind_of(expr_id) {
        Some(TypedExprKind::Ref(_)) => match typed.exprs.ty_of(expr_id) {
            Some(ty) if ty.is_mutable_ref() => ArgumentMode::Mutate,
            _ => ArgumentMode::Observe,
        },
        _ => ArgumentMode::Own,
    }
}

fn is_addressable_place(typed: &TypedFileAst, expr_id: ExprId) -> bool {
    match typed.exprs.kind_of(expr_id) {
        Some(TypedExprKind::FnCall { args, .. }) => {
            args.as_ref().is_none_or(|args| args.is_empty())
        }
        Some(TypedExprKind::AggregateGet { base, .. }) => is_addressable_place(typed, *base),
        Some(TypedExprKind::BufGet { buf, .. }) => is_addressable_place(typed, *buf),
        Some(TypedExprKind::Deref(_)) => true,
        _ => false,
    }
}

fn expected_mode(convention: ParamConvention) -> ArgumentMode {
    match convention {
        ParamConvention::Terminal => ArgumentMode::Own,
        ParamConvention::Own => ArgumentMode::Own,
        ParamConvention::Observe => ArgumentMode::Observe,
        ParamConvention::Mutate => ArgumentMode::Mutate,
    }
}

fn mode_name(mode: ArgumentMode) -> &'static str {
    match mode {
        ArgumentMode::Own => "owned",
        ArgumentMode::Observe => "ref",
        ArgumentMode::Mutate => "mut",
    }
}

fn collect_returning_sequence(
    typed: &TypedFileAst,
    exprs: &[ExprId],
    ret: Option<ExprId>,
    returns: &mut Vec<ExprId>,
) {
    for (index, expr) in exprs.iter().enumerate() {
        if let Some(TypedExprKind::If(branch)) = typed.exprs.kind_of(*expr)
            && branch.place_joins.is_empty()
        {
            if let Some((last, stmts)) = branch.stmts.split_last() {
                collect_returning_sequence(typed, stmts, Some(*last), returns);
            }
            if let Some(ret) = branch.ret {
                collect_return_values(typed, ret, returns);
            } else {
                collect_returning_sequence(typed, &exprs[index + 1..], ret, returns);
            }
            return;
        }
    }
    if let Some(ret) = ret {
        collect_return_values(typed, ret, returns);
    }
}

fn collect_return_values(typed: &TypedFileAst, expr: ExprId, returns: &mut Vec<ExprId>) {
    match typed.exprs.kind_of(expr) {
        Some(TypedExprKind::When(branch)) => {
            for arm in &branch.arms {
                let body = match arm {
                    crate::typed::TypedWhenArm::Cond { body, .. }
                    | crate::typed::TypedWhenArm::Is { body, .. }
                    | crate::typed::TypedWhenArm::Else(body, _) => *body,
                };
                collect_return_values(typed, body, returns);
            }
        }
        Some(TypedExprKind::Bind { stmts, body, .. }) => {
            collect_returning_sequence(typed, stmts, Some(*body), returns);
        }
        Some(TypedExprKind::If(branch)) => {
            if branch.place_joins.is_empty() {
                collect_returning_sequence(typed, &[expr], None, returns);
            } else if let Some(ret) = branch.ret {
                collect_return_values(typed, ret, returns);
            }
        }
        _ => returns.push(expr),
    }
}

fn validate_reference_returns(typed: &mut TypedFileAst) {
    let mut checks = Vec::new();
    for bind in typed.defs.values() {
        let signature = crate::typed::TypedCallableSignature::from(bind);
        let mut returns = Vec::new();
        match &bind.body {
            BindBody::Expr(expr) => collect_return_values(typed, *expr, &mut returns),
            BindBody::Body { exprs, ret } => {
                collect_returning_sequence(typed, exprs, *ret, &mut returns);
            }
            BindBody::Unlowered => continue,
        }
        for expr in returns {
            for (index, result) in signature.results.iter().enumerate() {
                if crate::ty::reference_semantics_for_type(&result.ty, Some(&typed.type_registry))
                    .is_none()
                {
                    continue;
                }
                let (site, actual) = match typed.exprs.kind_of(expr) {
                    Some(TypedExprKind::ResultRow(fields)) => {
                        let Some(field) = fields.get(index) else {
                            continue;
                        };
                        (
                            *field,
                            typed.exprs.target_group_of(*field).cloned().flatten(),
                        )
                    }
                    _ if signature.results.len() > 1 => (
                        expr,
                        typed
                            .exprs
                            .result_target_groups_of(expr)
                            .and_then(|groups| groups.get(index))
                            .cloned()
                            .flatten(),
                    ),
                    _ => (expr, typed.exprs.target_group_of(expr).cloned().flatten()),
                };
                let expected = bind.result_groups.get(index).copied().flatten();
                checks.push((site, expected, actual));
            }
        }
    }

    for (expr, expected, actual) in checks {
        let (code, message) = match actual.as_ref() {
            Some(targets)
                if expected.is_some()
                    && targets.iter().next().is_some()
                    && targets.iter().all(|target| target.root_param() == expected) =>
            {
                continue;
            }
            Some(targets) if targets.iter().any(|target| target.root_param().is_none()) => (
                "type-reference-return-local-target",
                "cannot return a reference to local storage",
            ),
            Some(targets) if expected.is_some() && targets.iter().next().is_some() => (
                "type-reference-return-wrong-target-group",
                "returned reference comes from the wrong target group",
            ),
            _ => (
                "type-reference-return-unknown-target",
                "cannot prove the returned reference outlives the call",
            ),
        };
        let mut diagnostic = Diagnostic::new(code, message);
        if let Some(span) = typed.exprs.span_of(expr) {
            diagnostic = diagnostic.at_span_id(span, &typed.span_table);
        }
        diagnostic = match code {
            "type-reference-return-local-target" => diagnostic
                .with_help_on_span("this reference escapes the local storage's lifetime")
                .with_help("return the value by ownership instead, or return a reference to caller-owned storage"),
            "type-reference-return-wrong-target-group" => diagnostic
                .with_help_on_span("this reference comes from a different input target group")
                .with_help("return a reference from the declared result target group"),
            _ => diagnostic
                .with_help_on_span("the lifetime of this reference cannot be established")
                .with_help("return an owned value, or a reference with a known input target group"),
        };
        if let Some(targets) = actual {
            let mut locals: Vec<_> = targets
                .iter()
                .filter_map(|target| target.root_local())
                .collect();
            locals.sort_by_key(|place| place.0);
            locals.dedup();
            for local in locals {
                let Some(place) = typed.places.get(local.0 as usize) else {
                    continue;
                };
                let span = typed
                    .defs
                    .values()
                    .find(|bind| bind.dependent_binder == place.binder)
                    .map(|bind| bind.name_span)
                    .or_else(|| {
                        typed.exprs.place_entries().find_map(|(expr, candidate)| {
                            (candidate == Some(local)
                                && matches!(
                                    typed.exprs.kind_of(expr),
                                    Some(TypedExprKind::Bind { .. })
                                ))
                            .then(|| typed.exprs.span_of(expr))
                            .flatten()
                        })
                    });
                if let Some(span) = span {
                    diagnostic.related.push(RelatedSpan {
                        span: typed.span_table.get(span),
                        label: format!(
                            "local `{}` is declared here; its storage does not outlive this call",
                            place.name
                        ),
                    });
                }
            }
        }
        typed.exprs.facts.semantic_facts.flaws[expr.index()].push(diagnostic);
    }
}

pub fn stage_validate_consumption(typed: &mut TypedFileAst) {
    validate_reference_returns(typed);

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
                        && let Some(TypedExprKind::Ref(inner)) = typed.exprs.kind_of(*arg_id)
                        && !is_addressable_place(typed, *inner)
                    {
                        typed.exprs.facts.semantic_facts.flaws[idx].push(
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
                let code = "type-argument-mode-mismatch";
                typed.exprs.facts.semantic_facts.flaws[idx].push(
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
