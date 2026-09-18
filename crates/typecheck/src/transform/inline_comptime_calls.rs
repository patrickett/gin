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

pub(super) fn stage_inline_package_calls(
    files: &mut [TypedFileAst],
    target: &flask::CompileTarget,
) {
    let programs: Vec<_> = files.iter().map(TypedFileAst::resolved_program).collect();
    let mut replacements = Vec::new();
    for (file_index, program) in programs.iter().copied().enumerate() {
        for (index, kind) in program.exprs.kind.iter().enumerate() {
            let TypedExprKind::FnCall {
                target: callable,
                args,
                ..
            } = kind
            else {
                continue;
            };
            let Some(bind) = programs
                .iter()
                .find_map(|program| program.defs.get(callable))
            else {
                continue;
            };
            if args.is_none() {
                continue;
            }
            if bind.call_capability == CallCapability::RuntimeOnly
                || bind.effects.unknown_memory
                || !bind.effects.reads.is_empty()
                || !bind.effects.writes.is_empty()
                || !bind.effects.invalidates.is_empty()
                || !bind.effects.consumes.is_empty()
            {
                continue;
            }
            let id = ExprId(index as u32);
            let mut evaluator =
                crate::eval::Evaluator::new(program, crate::eval::CompileTimeHost::new(target))
                    .with_programs(programs.iter().copied());
            if let Ok(crate::eval::EvalOutcome::Complete(
                value @ (ConstValue::Int(_) | ConstValue::Float(_) | ConstValue::Text(_)),
            )) = evaluator.eval(id)
            {
                replacements.push((file_index, id, value));
            }
        }
    }
    for (file_index, id, value) in replacements {
        replace_with_value(&mut files[file_index], id, value);
        files[file_index].exprs.facts.semantic_facts.availability[id.as_usize()] =
            Availability::CompileTime;
    }
}

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
        let params: Vec<_> = typed
            .defs
            .get(&def_id)
            .into_iter()
            .flat_map(|bind| bind.params.iter().map(|(name, _)| *name))
            .collect();
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
                changed |= inline_in_expr(typed, *root, &params);
            }
            if !changed {
                break;
            }
        }
    }
}

fn inline_in_expr(typed: &mut TypedFileAst, expr_id: ExprId, params: &[Intern<String>]) -> bool {
    let idx = expr_id.index();
    if idx >= typed.exprs.kind.len() {
        return false;
    }

    if matches!(typed.exprs.kind_of(expr_id), Some(TypedExprKind::FnCall { target, args, .. }) if args.as_ref().is_none_or(Vec::is_empty) && (params.contains(&target.0) || typed.exprs.place_of(expr_id).flatten().is_some()))
    {
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
        changed |= inline_in_expr(typed, child, params);
    }

    let TypedExprKind::FnCall { target, args, .. } = &root_kind else {
        return changed;
    };

    let Some(bind) = typed.defs.get(target) else {
        return changed;
    };
    if !is_staging_bind(bind) {
        return changed;
    }
    let args = match args {
        Some(args) => args.as_slice(),
        None => return changed,
    };

    if has_invalid_compile_time_argument(typed, bind, args) {
        emit_cannot_inline(typed, expr_id, &target.0, "non-constant argument");
        return changed;
    }

    match normalize(typed, expr_id, &crate::subst::DepSubst::new()) {
        crate::staging::NormalizeResult::Value(value) => {
            if !matches!(
                value,
                ConstValue::Text(_) | ConstValue::Int(_) | ConstValue::Float(_)
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
    typed.exprs.facts.const_facts.const_value[idx] = Some(value);
}

fn const_value_to_ty(value: &ConstValue) -> Option<crate::ty::Ty> {
    match value {
        ConstValue::Text(value) => Some(ast::Ty::Array {
            elem: Box::new(ast::Ty::u8()),
            size: ast::NormalExpr::from(value.len() as i128),
        }),
        ConstValue::Int(_) => None,
        ConstValue::Float(value) => Some(crate::ty::Ty::Float {
            value: Some(*value),
        }),
        ConstValue::ResultAlternative { .. }
        | ConstValue::Tag { .. }
        | ConstValue::Record { .. }
        | ConstValue::ResultRow(_)
        | ConstValue::FixedArray(_)
        | ConstValue::Sequence(_)
        | ConstValue::SequenceRepeat { .. } => {
            Some(crate::ty::Ty::Opaque(Intern::from_ref("comptime")))
        }
    }
}

fn emit_cannot_inline(
    typed: &mut TypedFileAst,
    expr_id: ExprId,
    fn_name: &Intern<String>,
    detail: &str,
) {
    let idx = expr_id.index();
    if typed.exprs.facts.semantic_facts.flaws[idx]
        .iter()
        .any(|flaw| flaw.code.slug() == "type-cannot-call-comptime-with-runtime-args")
    {
        return;
    }
    typed.exprs.facts.semantic_facts.flaws[idx].push(
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
        ConstValue::Text(value) => Literal::String(value.clone()),
        ConstValue::Int(value) => Literal::Int(*value),
        ConstValue::Float(HashFloat(value)) => Literal::Float(HashFloat(*value)),
        ConstValue::ResultAlternative { .. }
        | ConstValue::Tag { .. }
        | ConstValue::Record { .. }
        | ConstValue::ResultRow(_)
        | ConstValue::FixedArray(_)
        | ConstValue::Sequence(_)
        | ConstValue::SequenceRepeat { .. } => Literal::Number(0),
    }
}
#[cfg(test)]
#[path = "../../tests/inline_comptime_calls_tests.rs"]
mod tests;
