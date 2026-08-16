use std::collections::HashSet;

use crate::subst::DepSubst;
use crate::ty::{ParamKind, Ty};
use crate::typed::{
    Availability, AvailabilityRequirement, BindBody, CallCapability, ExprId, TypedExprKind,
    TypedFileAst, walk_expr_children_of,
};
use diagnostic::Diagnostic;

use super::{NormalizeResult, normalize};

/// Populate compile-time values from resolved expressions.
///
/// This is deliberately separate from parse preparation: availability and
/// normalization operate on the typed expression arena and therefore see the
/// same calls, substitutions, and effects as later compiler stages.
pub fn prepare_resolved_expressions(typed: &mut TypedFileAst) {
    validate_resolved_staging_calls(typed);
    validate_resolved_compile_time_binds(typed);

    let values: Vec<_> = (0..typed.exprs.kind.len())
        .map(|index| {
            if typed.exprs.availability[index] != Availability::CompileTime {
                return None;
            }
            if matches!(
                typed.exprs.kind[index],
                TypedExprKind::FnCall { .. } | TypedExprKind::Bind { .. }
            ) {
                return None;
            }

            match normalize(typed, ExprId(index as u32), &DepSubst::new()) {
                NormalizeResult::Value(mut value) => {
                    if let ast::ConstValue::Tag { qual_path, .. } = &mut value
                        && qual_path.is_none()
                        && let Ty::Union { name, .. } = &typed.exprs.ty[index]
                    {
                        *qual_path = Some(name.as_str().to_string());
                    }
                    Some(value)
                }
                NormalizeResult::Residual(_)
                | NormalizeResult::RuntimeDependency(_)
                | NormalizeResult::Unsupported(_) => None,
            }
        })
        .collect();

    for (index, value) in values.into_iter().enumerate() {
        if let Some(value) = value {
            typed.exprs.const_value[index] = Some(value);
        }
    }
}

fn validate_resolved_compile_time_binds(typed: &mut TypedFileAst) {
    let candidates: Vec<_> = typed
        .defs
        .values()
        .filter(|bind| is_resolved_compile_time_bind(bind))
        .map(|bind| {
            let roots = match &bind.body {
                BindBody::Expr(expr) => vec![*expr],
                BindBody::Body { exprs, ret } => {
                    exprs.iter().copied().chain(ret.iter().copied()).collect()
                }
                BindBody::Extern => Vec::new(),
            };
            (bind.name, roots)
        })
        .collect();

    for (bind_name, roots) in candidates {
        let Some(runtime_expr) = roots
            .into_iter()
            .find_map(|root| first_runtime_expr(typed, root))
        else {
            continue;
        };
        let index = runtime_expr.index();
        if typed.exprs.flaws[index]
            .iter()
            .any(|flaw| flaw.code.slug() == "compile-time-runtime-call")
        {
            continue;
        }
        typed.exprs.flaws[index].push(
            Diagnostic::new(
                "compile-time-runtime-call",
                format!("runtime call in compile-time bind `{}`", bind_name.as_str()),
            )
            .with_arg("bind_name", bind_name.as_str().to_string()),
        );
    }
}

fn is_resolved_compile_time_bind(bind: &crate::typed::TypedBind) -> bool {
    bind.is_constant
        && !bind.param_kinds.is_empty()
        && bind
            .param_kinds
            .iter()
            .all(|kind| matches!(kind, ParamKind::Type))
}

fn first_runtime_expr(typed: &TypedFileAst, root: ExprId) -> Option<ExprId> {
    let mut pending = vec![root];
    let mut visited = HashSet::new();
    while let Some(expr) = pending.pop() {
        if !visited.insert(expr) {
            continue;
        }
        if *typed
            .exprs
            .availability_of(expr)
            .unwrap_or(&Availability::Unknown)
            == Availability::Runtime
        {
            return Some(expr);
        }
        let _ = walk_expr_children_of(typed, expr, &mut |child| {
            pending.push(child);
            std::ops::ControlFlow::Continue(())
        });
    }
    None
}

fn validate_resolved_staging_calls(typed: &mut TypedFileAst) {
    for index in 0..typed.exprs.kind.len() {
        let TypedExprKind::FnCall {
            target,
            args: Some(args),
            ..
        } = typed.exprs.kind[index].clone()
        else {
            continue;
        };
        let Some(bind) = typed.defs.get(&target) else {
            continue;
        };
        if bind.call_capability != CallCapability::StagePolymorphic {
            continue;
        }

        let invalid = args
            .iter()
            .zip(&bind.param_requirements)
            .any(|(arg, requirement)| {
                *requirement == AvailabilityRequirement::CompileTime
                    && (typed
                        .exprs
                        .availability_of(*arg)
                        .unwrap_or(&Availability::Unknown)
                        != &Availability::CompileTime
                        || is_runtime_form_type_argument(typed, *arg))
            });
        if invalid
            && !typed.exprs.flaws[index]
                .iter()
                .any(|flaw| flaw.code.slug() == "type-cannot-call-comptime-with-runtime-args")
        {
            typed.exprs.flaws[index].push(
                Diagnostic::new(
                    "type-cannot-call-comptime-with-runtime-args",
                    format!(
                        "cannot call comptime function `{}` with runtime arguments",
                        bind.name.as_str()
                    ),
                )
                .with_arg("fn_name", bind.name.as_str().to_string())
                .with_arg("detail", "non-constant argument"),
            );
        }
    }
}

fn is_runtime_form_type_argument(typed: &TypedFileAst, arg: ExprId) -> bool {
    let Some(TypedExprKind::FnCall { target, .. }) = typed.exprs.kind_of(arg) else {
        return false;
    };
    if target.0.as_str() == "Self" {
        return false;
    }
    if !typed.defs.contains_key(target)
        && typed.exprs.availability_of(arg) == Some(&Availability::CompileTime)
    {
        return false;
    }
    !typed.defs.get(target).is_some_and(|bind| {
        bind.call_capability == CallCapability::StagePolymorphic
            && bind
                .param_kinds
                .iter()
                .all(|kind| matches!(kind, ParamKind::Type))
    })
}
