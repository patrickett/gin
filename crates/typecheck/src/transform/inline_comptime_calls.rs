use diagnostic::Diagnostic;
use internment::Intern;

use super::TransformCtx;
use ast::expr::Literal;
use ast::{ConstValue, FileAst, HashFloat};

use crate::staging::normalize;
use crate::typed::{
    Availability, AvailabilityRequirement, BindBody, CallCapability, DefId, ExprId, TypedExprKind,
    TypedFileAst, walk_expr_children_of,
};

const MAX_INLINE_ROUNDS: usize = 32;

pub fn stage_inline_comptime_calls(
    typed: &mut TypedFileAst,
    _file_ast: &FileAst,
    _ctx: &TransformCtx,
) {
    let runtime_defs: Vec<DefId> = typed
        .defs
        .iter()
        .filter(|(_, bind)| !bind.is_constant || bind.name.as_str() == "main")
        .map(|(id, _)| *id)
        .collect();

    for def_id in runtime_defs {
        let root_ids = match typed.defs.get(&def_id).map(|bind| &bind.body) {
            Some(BindBody::Expr(expr)) => vec![*expr],
            Some(BindBody::Body { exprs, ret }) => {
                let mut ids = exprs.clone();
                ids.extend(ret);
                ids
            }
            _ => continue,
        };

        for _ in 0..MAX_INLINE_ROUNDS {
            let mut changed = false;
            for root in &root_ids {
                changed |= inline_in_expr(typed, *root);
            }
            if !changed {
                break;
            }
        }
    }
}

fn inline_in_expr(typed: &mut TypedFileAst, expr_id: ExprId) -> bool {
    let idx = expr_id.index();
    if idx >= typed.exprs.kind.len() {
        return false;
    }

    let root_kind = match typed.exprs.kind_of(expr_id) {
        Some(kind) => kind.clone(),
        None => return false,
    };
    if let TypedExprKind::FnCall {
        target,
        args: Some(args),
        ..
    } = &root_kind
        && let Some(bind) = typed.defs.get(target)
        && is_staging_bind(bind)
        && bind.call_capability == CallCapability::StagePolymorphic
        && has_invalid_compile_time_argument(typed, bind, args)
    {
        emit_cannot_inline(typed, expr_id, &target.0, "non-constant argument");
        return false;
    }

    let mut children = Vec::new();
    let _ = walk_expr_children_of(typed, expr_id, &mut |child| {
        children.push(child);
        std::ops::ControlFlow::Continue(())
    });
    let mut changed = false;
    for child in children {
        changed |= inline_in_expr(typed, child);
    }

    let TypedExprKind::FnCall {
        target,
        args: Some(args),
        ..
    } = &root_kind
    else {
        return changed;
    };

    let Some(bind) = typed.defs.get(target) else {
        return changed;
    };
    if !is_staging_bind(bind) {
        return changed;
    }

    if has_invalid_compile_time_argument(typed, bind, args) {
        emit_cannot_inline(typed, expr_id, &target.0, "non-constant argument");
        return changed;
    }

    match normalize(typed, expr_id, &crate::subst::DepSubst::new()) {
        crate::staging::NormalizeResult::Value(value) => {
            if !matches!(
                value,
                ConstValue::String(_) | ConstValue::Int(_) | ConstValue::Float(_)
            ) {
                return changed;
            }
            replace_with_value(typed, expr_id, value);
            true
        }
        crate::staging::NormalizeResult::Unsupported(_) => {
            emit_cannot_inline(
                typed,
                expr_id,
                &target.0,
                "could not evaluate at compile time",
            );
            changed
        }
        crate::staging::NormalizeResult::Residual(_)
        | crate::staging::NormalizeResult::RuntimeDependency(_) => changed,
    }
}

fn is_staging_bind(bind: &crate::typed::TypedBind) -> bool {
    (bind.is_constant || bind.attributes.inline_always)
        && bind.call_capability == CallCapability::StagePolymorphic
}

fn has_invalid_compile_time_argument(
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
                .all(|kind| matches!(kind, crate::ty::ParamKind::Type))
    })
}

fn replace_with_value(typed: &mut TypedFileAst, expr_id: ExprId, value: ConstValue) {
    let idx = expr_id.as_usize();
    typed.exprs.kind[idx] = TypedExprKind::Lit(const_value_to_literal(&value));
    if let Some(ty) = const_value_to_ty(&value) {
        typed.exprs.ty[idx] = ty;
    }
    typed.exprs.const_value[idx] = Some(value);
}

fn const_value_to_ty(value: &ConstValue) -> Option<crate::ty::Ty> {
    match value {
        ConstValue::String(value) => {
            Some(crate::ty::Ty::Literal(ConstValue::String(value.clone())))
        }
        ConstValue::Int(_) => None,
        ConstValue::Float(value) => Some(crate::ty::Ty::Float {
            value: Some(*value),
        }),
        ConstValue::ResultAlternative { .. }
        | ConstValue::Tag { .. }
        | ConstValue::Record { .. }
        | ConstValue::List(_) => Some(crate::ty::Ty::Opaque(Intern::from_ref("comptime"))),
    }
}

fn emit_cannot_inline(
    typed: &mut TypedFileAst,
    expr_id: ExprId,
    fn_name: &Intern<String>,
    detail: &str,
) {
    let idx = expr_id.index();
    if typed.exprs.flaws[idx]
        .iter()
        .any(|flaw| flaw.code.slug() == "type-cannot-call-comptime-with-runtime-args")
    {
        return;
    }
    typed.exprs.flaws[idx].push(
        Diagnostic::new(
            "type-cannot-call-comptime-with-runtime-args",
            format!(
                "cannot call comptime function `{}` with runtime arguments: {}",
                fn_name.as_str(),
                detail
            ),
        )
        .with_arg("fn_name", fn_name.to_string())
        .with_arg("detail", detail.to_string()),
    );
}

fn const_value_to_literal(value: &ConstValue) -> Literal {
    match value {
        ConstValue::String(value) => Literal::String(value.clone()),
        ConstValue::Int(value) => Literal::Int(*value),
        ConstValue::Float(HashFloat(value)) => Literal::Float(HashFloat(*value)),
        ConstValue::ResultAlternative { .. }
        | ConstValue::Tag { .. }
        | ConstValue::Record { .. }
        | ConstValue::List(_) => Literal::Number(0),
    }
}
#[cfg(test)]
#[path = "../../tests/inline_comptime_calls_tests.rs"]
mod tests;
