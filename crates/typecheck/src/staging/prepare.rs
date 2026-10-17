use crate::subst::DepSubst;
use crate::ty::{ParamKind, Ty};
use crate::typed::{
    Availability, AvailabilityRequirement, BindBody, CallCapability, ExprId, TypedExprKind,
    TypedFileAst, walk_expr_preorder,
};
use diagnostic::code::{CallCode, EvaluationCode, TypeCode};
use diagnostic::{Diagnostic, DiagnosticCode};

use super::NormalizeResult;
use super::normalize::Normalizer;

/// Populate compile-time values from resolved expressions.
///
/// This is deliberately separate from parse preparation: availability and
/// normalization operate on the typed expression arena and therefore see the
/// same calls, substitutions, and effects as later compiler stages.
pub fn prepare_resolved_expressions(typed: &mut TypedFileAst) {
    let _profile = compiler_profile::span(compiler_profile::Phase::Staging);
    validate_resolved_staging_calls(typed);
    validate_resolved_compile_time_binds(typed);

    let normalizer = Normalizer::new(typed.resolved_program());
    let values: Vec<_> = (0..typed.exprs.kind.len())
        .map(|index| {
            if typed.exprs.facts.semantic_facts.availability[index] != Availability::CompileTime {
                return None;
            }
            if matches!(
                typed.exprs.kind[index],
                TypedExprKind::FnCall { .. } | TypedExprKind::Bind { .. }
            ) {
                return None;
            }

            match normalizer.normalize(ExprId(index as u32), &DepSubst::new()) {
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
            typed.exprs.facts.const_facts.const_value[index] = Some(value);
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
                BindBody::Unlowered => Vec::new(),
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
        if typed.exprs.facts.semantic_facts.flaws[index]
            .iter()
            .any(|flaw| flaw.code == DiagnosticCode::Evaluation(EvaluationCode::RuntimeCall))
        {
            continue;
        }
        typed.exprs.facts.semantic_facts.flaws[index].push(
            // compile-time-runtime-call
            Diagnostic::new(
                EvaluationCode::RuntimeCall,
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
    let mut first = None;
    let _ = walk_expr_preorder(typed, root, &mut |expr| {
        let runtime = *typed
            .exprs
            .availability_of(expr)
            .unwrap_or(&Availability::Unknown)
            == Availability::Runtime
            || matches!(
                typed.exprs.kind_of(expr),
                Some(TypedExprKind::FnCall { target, .. })
                    if typed.defs.get(target).is_some_and(|bind| !bind.is_constant)
            );
        if runtime {
            first = Some(expr);
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    });
    first
}

fn validate_resolved_staging_calls(typed: &mut TypedFileAst) {
    for index in 0..typed.exprs.kind.len() {
        if let TypedExprKind::Emit { specification, .. } = typed.exprs.kind[index].clone()
            && typed.exprs.availability_of(specification) != Some(&Availability::CompileTime)
            && !typed
                .exprs
                .const_value_of(specification)
                .is_some_and(Option::is_some)
            && !typed.exprs.facts.semantic_facts.flaws[index]
                .iter()
                .any(|flaw| {
                    flaw.code
                        == DiagnosticCode::Type(TypeCode::Call(
                            CallCode::CompileTimeRequiresConstantArguments,
                        ))
                })
        {
            typed.exprs.facts.semantic_facts.flaws[index].push(
                // type-cannot-call-comptime-with-runtime-args
                Diagnostic::new(
                    CallCode::CompileTimeRequiresConstantArguments,
                    "instruction encoding must be available at compile time",
                )
                .with_arg("fn_name", "#emit")
                .with_arg("detail", "runtime instruction encoding"),
            );
        }
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

        let invalid = has_invalid_compile_time_argument(typed, bind, &args);
        if invalid
            && !typed.exprs.facts.semantic_facts.flaws[index]
                .iter()
                .any(|flaw| {
                    flaw.code
                        == DiagnosticCode::Type(TypeCode::Call(
                            CallCode::CompileTimeRequiresConstantArguments,
                        ))
                })
        {
            typed.exprs.facts.semantic_facts.flaws[index].push(
                // type-cannot-call-comptime-with-runtime-args
                Diagnostic::new(
                    CallCode::CompileTimeRequiresConstantArguments,
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

pub(crate) fn has_invalid_compile_time_argument(
    typed: &TypedFileAst,
    bind: &crate::typed::TypedBind,
    args: &[ExprId],
) -> bool {
    args.iter()
        .zip(&bind.param_requirements)
        .any(|(arg, requirement)| {
            *requirement == AvailabilityRequirement::CompileTime
                && (typed
                    .exprs
                    .availability_of(*arg)
                    .unwrap_or(&Availability::Unknown)
                    != &Availability::CompileTime
                    || is_runtime_form_type_argument(typed, *arg))
        })
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
