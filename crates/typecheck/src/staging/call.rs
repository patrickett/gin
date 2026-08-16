use ast::NormalExpr;
use ast::ty::{IntegerInterpretation, ParamKind, TyArg};

use crate::analysis::unify_type_args_with_registry;
use crate::subst::DepSubst;
use crate::ty::Ty;
use crate::typed::{CallCapability, ExprId, TypedExprKind, TypedFileAst};
use internment::Intern;

pub struct CallInstantiation {
    pub args: Option<Vec<ExprId>>,
    pub substituted_ty: Option<Ty>,
}

pub fn instantiate_call(
    raw_args: &[ExprId],
    typed: &TypedFileAst,
    param_kinds: &[ParamKind],
    params: &[(Intern<String>, Ty)],
    return_type: &Ty,
) -> CallInstantiation {
    if param_kinds.len() != raw_args.len() {
        return CallInstantiation {
            args: Some(raw_args.to_vec()),
            substituted_ty: None,
        };
    }

    let ordered_args = reorder_args_by_name(raw_args, typed, params);
    let ordered_args = if let Some(args) = ordered_args {
        args
    } else {
        return CallInstantiation {
            args: Some(raw_args.to_vec()),
            substituted_ty: None,
        };
    };

    let mut lowered_args = Vec::with_capacity(ordered_args.len());
    let mut type_args = Vec::with_capacity(ordered_args.len());

    for (index, arg) in ordered_args.iter().enumerate() {
        let arg = call_arg_value_expr(typed, *arg);
        let kind = &param_kinds[index];
        let param_ty = &params[index].1;
        match kind {
            ParamKind::Type => {
                if type_arg_is_compile_time(typed, arg) {
                    let arg_ty = typed.exprs.ty[arg.index()].clone();
                    type_args.push(TyArg::Type(Box::new(arg_ty)));
                } else {
                    lowered_args.push(arg);
                }
            }
            ParamKind::Value(_) => {
                lowered_args.push(arg);
                if value_param_ty_is_inferred(param_ty) {
                    let arg_ty = typed.exprs.ty[arg.index()].clone();
                    type_args.push(TyArg::Type(Box::new(normalize_inferred_value_type(
                        param_ty, &arg_ty,
                    ))));
                } else if let Some(value) = expr_to_normal_expr(typed, arg) {
                    type_args.push(TyArg::Const(value));
                }
            }
        }
    }

    let expected: Vec<TyArg> = param_kinds
        .iter()
        .enumerate()
        .map(|(i, kind)| match kind {
            ParamKind::Type => TyArg::Type(Box::new(Ty::Opaque(params[i].0))),
            ParamKind::Value(_) => {
                let param_ty = &params[i].1;
                if value_param_ty_is_inferred(param_ty) {
                    TyArg::Type(Box::new(param_ty.clone()))
                } else {
                    TyArg::Const(NormalExpr::Var(params[i].0))
                }
            }
        })
        .collect();

    CallInstantiation {
        args: Some(lowered_args),
        substituted_ty: if param_kinds.is_empty() {
            Some(return_type.clone())
        } else if type_args.len() != expected.len() {
            None
        } else {
            unify_type_args_with_registry(&expected, &type_args, Some(&typed.type_registry))
                .ok()
                .map(|subst| subst.apply_to_ty(return_type))
        },
    }
}

fn normalize_inferred_value_type(param_ty: &Ty, arg_ty: &Ty) -> Ty {
    if !value_param_ty_contains_generic_opaque(param_ty) {
        return arg_ty.clone();
    }

    match arg_ty {
        Ty::AnonymousInteger { .. } => Ty::anonymous_integer_for_width(
            64,
            arg_ty.anonymous_operation_interpretation() != Some(IntegerInterpretation::Unsigned),
        ),
        Ty::ResultFamily { .. } => arg_ty.clone(),
        _ => arg_ty.clone(),
    }
}

fn value_param_ty_contains_generic_opaque(ty: &Ty) -> bool {
    match ty {
        Ty::ResultFamily { .. } => false,
        Ty::Opaque(name) => name.as_str().chars().next().is_some_and(char::is_lowercase),
        Ty::Named { instance, .. } => instance.arguments.iter().any(|(_, arg)| match arg {
            ast::TyArg::Type(inner) => value_param_ty_contains_generic_opaque(inner),
            ast::TyArg::Const(_) => false,
        }),
        Ty::Record {
            fields,
            resolved_params,
            ..
        } => {
            fields
                .iter()
                .any(|(_, field_ty)| value_param_ty_contains_generic_opaque(field_ty))
                || resolved_params.as_ref().is_some_and(|params| {
                    params.iter().any(|(_, arg)| match arg {
                        ast::TyArg::Type(arg_ty) => value_param_ty_contains_generic_opaque(arg_ty),
                        ast::TyArg::Const(_) => false,
                    })
                })
        }
        Ty::Union {
            variants,
            resolved_params,
            ..
        } => {
            variants.iter().any(|variant| {
                variant
                    .fields
                    .iter()
                    .any(|(_, field_ty)| value_param_ty_contains_generic_opaque(field_ty))
                    || variant
                        .result_ty
                        .as_ref()
                        .is_some_and(value_param_ty_contains_generic_opaque)
            }) || resolved_params.as_ref().is_some_and(|params| {
                params.iter().any(|(_, arg)| match arg {
                    ast::TyArg::Type(arg_ty) => value_param_ty_contains_generic_opaque(arg_ty),
                    ast::TyArg::Const(_) => false,
                })
            })
        }
        Ty::Ref { inner, .. } | Ty::Ptr { inner } => value_param_ty_contains_generic_opaque(inner),
        Ty::Address { pointee, .. } => value_param_ty_contains_generic_opaque(pointee),
        Ty::Tuple(tys) => tys.iter().any(value_param_ty_contains_generic_opaque),
        Ty::Array { elem, .. } => value_param_ty_contains_generic_opaque(elem),
        Ty::Literal(_)
        | Ty::UnresolvedLiteral(_)
        | Ty::AnonymousInteger { .. }
        | Ty::Float { .. }
        | Ty::Unit => false,
    }
}

fn value_param_ty_is_inferred(param_ty: &Ty) -> bool {
    value_param_ty_contains_generic_opaque(param_ty)
}

fn expr_to_normal_expr(typed: &TypedFileAst, arg: ExprId) -> Option<NormalExpr> {
    typed
        .exprs
        .const_value_of(arg)
        .and_then(Clone::clone)
        .map(NormalExpr::Value)
        .or_else(|| {
            if matches!(
                typed.exprs.kind_of(arg),
                Some(TypedExprKind::SelfRef { .. })
            ) {
                Some(NormalExpr::Var(Intern::from_ref("Self")))
            } else {
                super::normalize::normalize(typed, arg, &DepSubst::new()).as_dependent_normal_expr()
            }
        })
}

fn type_arg_is_compile_time(typed: &TypedFileAst, arg: ExprId) -> bool {
    if let Some(TypedExprKind::FnCall { target, .. }) = typed.exprs.kind_of(arg)
        && target.0.as_str() == "Self"
    {
        return true;
    }
    if let Some(TypedExprKind::FnCall { target, .. }) = typed.exprs.kind_of(arg)
        && !typed.defs.get(target).is_some_and(|bind| {
            bind.call_capability == CallCapability::StagePolymorphic
                && bind
                    .param_kinds
                    .iter()
                    .all(|kind| matches!(kind, ParamKind::Type))
        })
    {
        return false;
    }

    !matches!(
        super::normalize::normalize(typed, arg, &DepSubst::new()),
        super::normalize::NormalizeResult::RuntimeDependency(_)
            | super::normalize::NormalizeResult::Unsupported(_)
    )
}

fn reorder_args_by_name(
    args: &[ExprId],
    typed: &TypedFileAst,
    params: &[(Intern<String>, Ty)],
) -> Option<Vec<ExprId>> {
    let has_named = args.iter().any(|arg| call_arg_name(typed, *arg).is_some());
    if !has_named {
        return Some(args.to_vec());
    }

    let mut name_to_index = std::collections::HashMap::new();
    for (index, (name, _)) in params.iter().enumerate() {
        name_to_index.insert(*name, index);
    }

    let mut reordered = vec![None; args.len()];
    let mut used_params = vec![false; args.len()];
    let mut positional: Vec<ExprId> = Vec::new();

    for &arg in args {
        if let Some(name) = call_arg_name(typed, arg) {
            let index = name_to_index.get(&name).copied()?;
            if used_params[index] {
                return None;
            }
            reordered[index] = Some(arg);
            used_params[index] = true;
        } else {
            positional.push(arg);
        }
    }

    let mut next_positional = 0;
    for slot in &mut reordered {
        if slot.is_none() {
            let arg = positional.get(next_positional).copied()?;
            *slot = Some(arg);
            next_positional += 1;
        }
    }

    if next_positional != positional.len() {
        return None;
    }

    reordered.into_iter().collect()
}

fn call_arg_name(typed: &TypedFileAst, arg: ExprId) -> Option<Intern<String>> {
    match typed.exprs.kind_of(arg) {
        Some(TypedExprKind::Bind { name, .. }) => Some(*name),
        _ => None,
    }
}

fn call_arg_value_expr(typed: &TypedFileAst, arg: ExprId) -> ExprId {
    match typed.exprs.kind_of(arg) {
        Some(TypedExprKind::Bind { body, .. }) => *body,
        _ => arg,
    }
}
