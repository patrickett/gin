//! Inline calls to comptime-classified functions at runtime call sites when all arguments are known.

use std::collections::{HashMap, HashSet};

use diagnostic::Diagnostic;
use internment::Intern;

use super::TransformCtx;
use super::flow::{children_of_kind, eval_const_from_expr};
use crate::analysis::eval_compile_time_bind_call;
use crate::reflect::reflect_ty_to_const_value;
use ast::BindValue;
use ast::FileAst;
use ast::expr::Literal;
use ast::{ConstValue, HashFloat};

use crate::ty::Ty;
use crate::typed::{BindBody, DefId, ExprId, TypedExprKind, TypedFileAst};

const MAX_INLINE_DEPTH: usize = 64;
const MAX_INLINE_ROUNDS: usize = 32;

/// Replace comptime `FnCall`s in runtime function bodies when arguments are compile-time-known.
pub fn stage_inline_comptime_calls(
    typed: &mut TypedFileAst,
    file_ast: &FileAst,
    ctx: &TransformCtx,
) {
    let eval_ast = ctx.compile_time_eval_ast.as_ref();
    let env = build_top_level_const_env(eval_ast, file_ast);

    let runtime_defs: Vec<DefId> = typed
        .defs
        .iter()
        .filter(|(_, b)| !b.is_compile_time)
        .map(|(id, _)| *id)
        .collect();

    for def_id in runtime_defs {
        let root_ids = match typed.defs.get(&def_id).map(|b| &b.body) {
            Some(BindBody::Expr(id)) => vec![*id],
            Some(BindBody::Body { exprs, ret }) => {
                let mut ids = exprs.clone();
                ids.extend(ret);
                ids
            }
            _ => continue,
        };
        for _round in 0..MAX_INLINE_ROUNDS {
            let mut changed = false;
            for root in &root_ids {
                if inline_in_expr(
                    typed,
                    eval_ast,
                    file_ast,
                    *root,
                    &env,
                    &mut HashSet::new(),
                    0,
                ) {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }
}

fn build_top_level_const_env(
    eval_ast: &FileAst,
    file_ast: &FileAst,
) -> HashMap<Intern<String>, Option<ConstValue>> {
    let mut env: HashMap<Intern<String>, Option<ConstValue>> = eval_ast
        .defs
        .keys()
        .chain(file_ast.defs.keys())
        .map(|n| (*n, None))
        .collect();

    for (name, bind) in file_ast.defs.iter().chain(eval_ast.defs.iter()) {
        if let BindValue::Expr(e) = &bind.value
            && let Some(cv) = &e.const_value
        {
            env.insert(*name, Some(cv.clone()));
        }
    }
    env
}

fn inline_in_expr(
    typed: &mut TypedFileAst,
    eval_ast: &FileAst,
    file_ast: &FileAst,
    expr_id: ExprId,
    env: &HashMap<Intern<String>, Option<ConstValue>>,
    call_stack: &mut HashSet<String>,
    depth: usize,
) -> bool {
    let idx = expr_id.as_usize();
    if idx >= typed.exprs.kind.len() {
        return false;
    }

    let mut changed = false;
    for child in children_of_kind(&typed.exprs.kind[idx]) {
        if inline_in_expr(typed, eval_ast, file_ast, child, env, call_stack, depth) {
            changed = true;
        }
    }

    let TypedExprKind::FnCall { target, args } = typed.exprs.kind[idx].clone() else {
        return changed;
    };

    let Some(bind) = eval_ast
        .defs
        .get(&target.0)
        .or_else(|| file_ast.defs.get(&target.0))
    else {
        return changed;
    };

    if !bind.is_compile_time {
        return changed;
    }

    let target_name = target.0.as_str().to_string();
    if call_stack.contains(&target_name) || depth >= MAX_INLINE_DEPTH {
        return changed;
    }

    let arg_ids = match args {
        Some(a) => a,
        None => {
            if bind.params.is_some() {
                emit_cannot_inline(typed, expr_id, &target.0, "missing arguments");
            }
            return changed;
        }
    };

    let arg_cvs = match collect_call_arg_const_values(typed, &arg_ids) {
        Some(v) => v,
        None => {
            emit_cannot_inline(typed, expr_id, &target.0, "non-constant argument");
            return changed;
        }
    };

    call_stack.insert(target_name.clone());
    let result = eval_compile_time_bind_call(bind, &arg_cvs, env, eval_ast, depth + 1, call_stack);
    call_stack.remove(&target_name);

    let Some(cv) = result else {
        emit_cannot_inline(
            typed,
            expr_id,
            &target.0,
            "could not evaluate at compile time",
        );
        return changed;
    };

    let lit = const_value_to_literal(&cv);
    typed.exprs.kind[idx] = TypedExprKind::Lit(lit.clone());
    // Tag values (e.g. `True`, `False`, `None`) keep their original type from the
    // function's return type annotation — don't reconstruct it from the ConstValue.
    if !matches!(&cv, ConstValue::Tag { .. }) {
        typed.exprs.ty[idx] = const_value_to_ty(&cv);
    }
    typed.exprs.const_value[idx] = Some(cv);
    true
}

fn collect_call_arg_const_values(
    typed: &TypedFileAst,
    arg_ids: &[ExprId],
) -> Option<Vec<ConstValue>> {
    let mut out = Vec::with_capacity(arg_ids.len());
    for id in arg_ids {
        out.push(arg_const_value(typed, id.as_usize())?);
    }
    Some(out)
}

fn arg_const_value(typed: &TypedFileAst, idx: usize) -> Option<ConstValue> {
    if let TypedExprKind::FnCall { target, .. } = &typed.exprs.kind[idx] {
        return typed
            .defs
            .get(target)
            .filter(|b| b.is_compile_time)
            .and_then(|_| typed.exprs.const_value[idx].clone());
    }
    if let Some(cv) = typed.exprs.const_value[idx].clone() {
        return Some(cv);
    }
    if let Some(cv) = eval_const_from_expr(typed, idx) {
        return Some(cv);
    }
    // Type / tag operands: `Int` in `f(Int)` is a `TagCall` with `ty: Opaque("Int")`.
    if matches!(
        &typed.exprs.kind[idx],
        TypedExprKind::Lit(_) | TypedExprKind::TagCall { .. }
    ) && !matches!(&typed.exprs.ty[idx], Ty::Literal(_))
    {
        return Some(reflect_ty_to_const_value(&typed.exprs.ty[idx]));
    }
    None
}

fn const_value_to_ty(cv: &ConstValue) -> Ty {
    match cv {
        ConstValue::String(s) => Ty::Literal(ConstValue::String(s.clone())),
        ConstValue::Int(n) => Ty::Int {
            width: 64,
            signed: true,
            value: Some(*n),
            min: None,
            max: None,
        },
        ConstValue::Float(HashFloat(f)) => Ty::Float {
            value: Some(HashFloat(*f)),
        },
        _ => Ty::Opaque(Intern::new("comptime".to_string())),
    }
}

fn emit_cannot_inline(
    typed: &mut TypedFileAst,
    expr_id: ExprId,
    fn_name: &Intern<String>,
    detail: &str,
) {
    typed.exprs.flaws[expr_id.as_usize()].push(
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

fn const_value_to_literal(cv: &ConstValue) -> Literal {
    match cv {
        ConstValue::String(s) => Literal::String(s.clone()),
        ConstValue::Int(n) => Literal::Int(u128::try_from(*n).unwrap_or(0)),
        ConstValue::Float(HashFloat(f)) => Literal::Float(HashFloat(*f)),
        ConstValue::Tag { .. } => Literal::Number(0),
        ConstValue::Record { .. } | ConstValue::List(_) => Literal::Number(0),
    }
}
