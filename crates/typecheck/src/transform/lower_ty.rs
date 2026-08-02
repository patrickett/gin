//! Type-resolution helpers used during expression lowering.
//!
//! Provides functions for resolving expression types, binding explicit
//! type annotations, converting literals to types, extracting element
//! types, annotating literal union values, resolving record field
//! indices, converting [`ConstValue`]s to [`Literal`]s, and resolving
//! function-call targets.

use internment::Intern;
use std::collections::HashMap;

use crate::analysis::{TyInfer, TyInferEnv, TypeEnv, expr_is_type_surface};
use ast::path::ModPath;
use ast::prelude::*;
use ast::{BinderId, NormalExpr, ConstValue, HashFloat, Parameters};

use crate::ty::Ty;
use crate::typed::{DefId, ExprId, TypedExprKind, TypedFileAst, VariantMap};

use super::lower_exprs::LocalVarTypes;
use super::lower_tag::resolve_tag_call_type;

/// Resolve the [`Ty`] of a typed parse expression by walking its AST and
/// applying type inference / tag resolution.
pub(crate) fn resolve_expr_type(
    expr: &Typed<Expr>,
    tag_types: &HashMap<Intern<String>, Ty>,
    variant_map: &VariantMap,
    receiver_type: Option<&Ty>,
    expected_ty: Option<&Ty>,
    local_var_types: &LocalVarTypes,
    fn_return_types: &HashMap<DefId, Ty>,
) -> Ty {
    if let Some(resolved) = expr.resolved_ty() {
        return resolved.clone();
    }
    match &expr.value {
        Expr::Lit(lit) => lit_to_ty(lit),
        Expr::Binary(bin) => {
            let returns: HashMap<Intern<String>, Ty> = fn_return_types
                .iter()
                .map(|(d, t)| (d.0, t.clone()))
                .collect();
            let env = TyInferEnv {
                tag_types,
                fn_return_types: &returns,
                locals: local_var_types,
                tag_params: None,
            };
            bin.infer_ty(&env)
        }
        Expr::FnCall(fn_call) => {
            let returns: HashMap<Intern<String>, Ty> = fn_return_types
                .iter()
                .map(|(d, t)| (d.0, t.clone()))
                .collect();
            let env = TyInferEnv {
                tag_types,
                fn_return_types: &returns,
                locals: local_var_types,
                tag_params: None,
            };
            fn_call.infer_ty(&env)
        }
        Expr::TagCall(tag_call) if tag_call.name.as_str() == "Self" => {
            receiver_type.cloned().unwrap_or(Ty::Opaque(tag_call.name))
        }
        Expr::TagCall(tag_call) => {
            resolve_tag_call_type(&tag_call.name, tag_types, variant_map, expected_ty)
        }
        Expr::AnonymousTag(name) if name.as_str() == "Self" => {
            receiver_type.cloned().unwrap_or(Ty::Opaque(*name))
        }
        Expr::AnonymousTag(name) => {
            resolve_tag_call_type(name, tag_types, variant_map, expected_ty)
        }
        Expr::Bind(local_bind) => {
            if let Some(ty) = bind_explicit_ty(local_bind, tag_types) {
                return ty;
            }

            match &local_bind.value {
                BindValue::Expr(e) => resolve_expr_type(
                    e,
                    tag_types,
                    variant_map,
                    receiver_type,
                    expected_ty,
                    local_var_types,
                    fn_return_types,
                ),
                BindValue::Body { exprs, ret } => exprs
                    .last()
                    .map(|e| {
                        resolve_expr_type(
                            e,
                            tag_types,
                            variant_map,
                            receiver_type,
                            None,
                            local_var_types,
                            fn_return_types,
                        )
                    })
                    .or_else(|| {
                        ret.value.as_ref().map(|e| {
                            resolve_expr_type(
                                e,
                                tag_types,
                                variant_map,
                                receiver_type,
                                expected_ty,
                                local_var_types,
                                fn_return_types,
                            )
                        })
                    })
                    .unwrap_or(Ty::Unit),
                BindValue::Extern | BindValue::Unassigned => Ty::Unit,
            }
        }
        Expr::When(when_expr) => when_expr
            .arms
            .first()
            .map(|arm| match arm {
                WhenArm::Cond { body, .. } | WhenArm::Is { body, .. } => resolve_expr_type(
                    body,
                    tag_types,
                    variant_map,
                    receiver_type,
                    expected_ty,
                    local_var_types,
                    fn_return_types,
                ),
                WhenArm::Else(body, _) => resolve_expr_type(
                    body,
                    tag_types,
                    variant_map,
                    receiver_type,
                    expected_ty,
                    local_var_types,
                    fn_return_types,
                ),
            })
            .unwrap_or_else(|| {
                expected_ty
                    .cloned()
                    .unwrap_or(Ty::Opaque(Intern::new("infer".to_string())))
            }),
        Expr::If(if_expr) => if_expr
            .body
            .first()
            .map(|e| {
                resolve_expr_type(
                    e,
                    tag_types,
                    variant_map,
                    receiver_type,
                    None,
                    local_var_types,
                    fn_return_types,
                )
            })
            .unwrap_or(Ty::Unit),
        Expr::Loop(_) => Ty::Unit,
        Expr::SelfRef => receiver_type
            .cloned()
            .unwrap_or_else(|| Ty::Opaque(Intern::new("self".to_string()))),
        Expr::FormatString(_) => Ty::Opaque(Intern::new("format_string".to_string())),
        Expr::Range(_) => Ty::Opaque(Intern::new("range_expr".to_string())),
        Expr::TupleAlloc { init, size } => {
            let elem_ty = resolve_expr_type(
                init,
                tag_types,
                variant_map,
                receiver_type,
                None,
                local_var_types,
                fn_return_types,
            );
            Ty::Array {
                elem: Box::new(elem_ty),
                size: expr_as_size_normal_expr(&size.value)
                    .unwrap_or_else(|| fallback_tuple_alloc_size(size)),
            }
        }
        Expr::TupleGet { base, .. } => extract_element_type(&resolve_expr_type(
            base,
            tag_types,
            variant_map,
            receiver_type,
            None,
            local_var_types,
            fn_return_types,
        )),
        Expr::Destructure { value, .. } => resolve_expr_type(
            value,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
        ),
        Expr::RecordSet { value, .. } => resolve_expr_type(
            value,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
        ),
        Expr::RecordGet { base, field } => {
            let base_ty = resolve_expr_type(
                base,
                tag_types,
                variant_map,
                receiver_type,
                None,
                local_var_types,
                fn_return_types,
            );
            match &base_ty {
                Ty::Record { fields, .. } => fields
                    .iter()
                    .find(|(n, _)| n == field)
                    .map(|(_, t)| t.as_ref().clone())
                    .unwrap_or_else(|| Ty::Opaque(*field)),
                _ => Ty::Opaque(*field),
            }
        }
        Expr::TupleSet { value, .. } => resolve_expr_type(
            value,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
        ),
        Expr::Cast { ty, .. } => tag_types.get(ty).cloned().unwrap_or(Ty::Opaque(*ty)),
        Expr::BufGet { buf, .. } => extract_element_type(&resolve_expr_type(
            buf,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
        )),
        Expr::BufSet { value, .. } => resolve_expr_type(
            value,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
        ),
        Expr::TakePtr(inner) => Ty::Ptr {
            inner: Box::new(resolve_expr_type(
                inner,
                tag_types,
                variant_map,
                receiver_type,
                expected_ty,
                local_var_types,
                fn_return_types,
            )),
        },
        Expr::Ref { inner, mutable, .. } => Ty::Ref {
            inner: Box::new(resolve_expr_type(
                inner,
                tag_types,
                variant_map,
                receiver_type,
                expected_ty,
                local_var_types,
                fn_return_types,
            )),
            mutable: *mutable,
        },
        Expr::Eat(inner) => resolve_expr_type(
            inner,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
        ),
        Expr::ConsumeArg(inner) => resolve_expr_type(
            inner,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
        ),
        Expr::Deref(inner) => {
            let inner_ty = resolve_expr_type(
                inner,
                tag_types,
                variant_map,
                receiver_type,
                expected_ty,
                local_var_types,
                fn_return_types,
            );
            match &inner_ty {
                Ty::Ptr { inner } => *inner.clone(),
                _ => Ty::Opaque(Intern::new("infer".to_string())),
            }
        }
        Expr::Negate(inner) => resolve_expr_type(
            inner,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
        ),
        Expr::RecordLit(fields) => Ty::Tuple(
            fields
                .iter()
                .map(|(_, item)| {
                    resolve_expr_type(
                        item,
                        tag_types,
                        variant_map,
                        receiver_type,
                        None,
                        local_var_types,
                        fn_return_types,
                    )
                })
                .collect(),
        ),
        Expr::TupleLit(items) => Ty::Tuple(
            items
                .iter()
                .map(|item| {
                    resolve_expr_type(
                        item,
                        tag_types,
                        variant_map,
                        receiver_type,
                        None,
                        local_var_types,
                        fn_return_types,
                    )
                })
                .collect(),
        ),
        Expr::List(items) => items
            .first()
            .map(|e| {
                resolve_expr_type(
                    e,
                    tag_types,
                    variant_map,
                    receiver_type,
                    None,
                    local_var_types,
                    fn_return_types,
                )
            })
            .unwrap_or(Ty::Opaque(Intern::new("List".to_string()))),
        Expr::Asm(_) => Ty::Unit,
    }
}

/// When a bind is annotated with a const union, give its literal the union type (not `Str`).
pub(crate) fn annotate_literal_union_literal(
    typed: &mut TypedFileAst,
    body: ExprId,
    union_ty: &Ty,
    values: &[ConstValue],
) {
    let idx = body.as_usize();
    if idx >= typed.exprs.kind.len() {
        return;
    }
    let TypedExprKind::Lit(_lit) = &typed.exprs.kind[idx] else {
        return;
    };
    // Preserve the literal's actual compile-time value when available; only force
    // a default when no concrete value is known yet.
    typed.exprs.ty[idx] = union_ty.clone();
    if typed.exprs.const_value[idx].is_none()
        && let Some(value) = values.first()
    {
        typed.exprs.const_value[idx] = Some(value.clone());
    }
}

/// Explicit type from `name Ty:` / `name Ty :=` (return tag or return type name).
pub(crate) fn nominalize_explicit_ty_surface(surface: &ast::Expr, ty: Ty) -> Ty {
    let ast::Expr::AnonymousTag(name) = surface else {
        return ty;
    };
    match ty {
        Ty::Record {
            name: existing,
            fields,
            resolved_params,
        } if existing.as_str() == "record" => Ty::Record {
            name: *name,
            fields,
            resolved_params,
        },
        Ty::Union {
            name: existing,
            variants,
            literal_values,
            ..
        } if existing.as_str() == "union" => Ty::Union {
            name: *name,
            variants,
            literal_values,
            resolved_params: None,
        },
        other => other,
    }
}

pub(crate) fn bind_local_explicit_ty(
    bind: &Bind,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    tag_decls: &ast::TagMap,
    binder: BinderId,
    initializer_ty: Option<&Ty>,
) -> Option<Ty> {
    let surface = bind
        .return_tag
        .as_ref()
        .map(|sp| &sp.value)
        .filter(|surface| expr_is_type_surface(surface));
    let Some(surface) = surface else {
        return bind_explicit_ty(bind, tag_types);
    };
    let resolved = TypeEnv::new(tag_types)
        .with_tag_params(tag_params)
        .with_tag_decls(tag_decls)
        .with_dependent_binder(binder)
        .resolve(surface);
    let resolved = nominalize_explicit_ty_surface(surface, resolved);

    match (initializer_ty, &resolved) {
        (
            Some(
                initializer @ Ty::Record {
                    name: initializer_name,
                    ..
                },
            ),
            Ty::Record {
                name: resolved_name,
                ..
            },
        ) if initializer_name == resolved_name => Some(initializer.clone()),
        (
            Some(
                initializer @ Ty::Union {
                    name: initializer_name,
                    ..
                },
            ),
            Ty::Union {
                name: resolved_name,
                ..
            },
        ) if initializer_name == resolved_name => Some(initializer.clone()),
        _ => Some(resolved),
    }
}

pub(crate) fn bind_explicit_ty(bind: &Bind, tag_types: &HashMap<Intern<String>, Ty>) -> Option<Ty> {
    if let Some(sp) = &bind.return_tag
        && expr_is_type_surface(&sp.value)
    {
        let ty = TypeEnv::new(tag_types).resolve(&sp.value);
        return Some(nominalize_explicit_ty_surface(&sp.value, ty));
    }
    if let Some(name) = &bind.return_type_name {
        return tag_types
            .get(name)
            .cloned()
            .map(|ty| {
                nominalize_explicit_ty_surface(
                    &ast::Expr::AnonymousTag(*name),
                    ty,
                )
            })
            .or(Some(Ty::Opaque(*name)));
    }
    None
}

/// Convert a parsed [`Literal`] to its corresponding [`Ty`].
pub(crate) fn lit_to_ty(lit: &Literal) -> Ty {
    match lit {
        Literal::Number(n) => Ty::Int {
            width: 64,
            signed: true,
            value: Some(*n as i128),
            min: None,
            max: None,
        },
        Literal::Float(HashFloat(f)) => Ty::Float {
            value: Some(HashFloat(*f)),
        },
        Literal::Int(n) => Ty::Int {
            width: 64,
            signed: false,
            value: Some(*n as i128),
            min: None,
            max: None,
        },
        Literal::String(s) => Ty::Literal(ConstValue::String(s.clone())),
    }
}

fn expr_as_size_normal_expr(expr: &Expr) -> Option<NormalExpr> {
    match expr {
        Expr::Lit(Literal::Int(n)) => Some(NormalExpr::from(*n as i128)),
        Expr::Lit(Literal::Number(n)) => Some(NormalExpr::from(*n as i128)),
        Expr::Bind(bind) => Some(NormalExpr::Var(bind.name)),
        Expr::Binary(binary) => {
            let lhs = expr_as_size_normal_expr(&binary.lhs.value)?;
            let rhs = expr_as_size_normal_expr(&binary.rhs.value)?;
            Some(match binary.op {
                ast::expr::BinOp::Add => NormalExpr::Add(Box::new(lhs), Box::new(rhs)),
                ast::expr::BinOp::Subtract => NormalExpr::Sub(Box::new(lhs), Box::new(rhs)),
                ast::expr::BinOp::Multiply => NormalExpr::Mul(Box::new(lhs), Box::new(rhs)),
                _ => return None,
            })
        }
        _ => None,
    }
}

fn fallback_tuple_alloc_size(expr: &Typed<Expr>) -> NormalExpr {
    NormalExpr::Var(Intern::new(format!(
        "_unsupported_array_size_{:?}",
        expr.span_id
    )))
}

/// Extract the element type from an array or tuple type.
pub(crate) fn extract_element_type(ty: &Ty) -> Ty {
    match ty {
        Ty::Array { elem, .. } => *elem.clone(),
        Ty::Tuple(fields) => fields.first().cloned().unwrap_or(Ty::Unit),
        _ => Ty::Opaque(Intern::new("infer".to_string())),
    }
}

/// Resolve a function-call path to a [`DefId`].
pub(crate) fn resolve_fn_call_target(
    path: &ModPath,
    _tag_types: &HashMap<Intern<String>, Ty>,
) -> DefId {
    let fq_name = if path.segments.is_empty() {
        path.root
    } else {
        let mut parts: Vec<&str> = vec![path.root.as_str()];
        for seg in &path.segments {
            parts.push(seg.as_str());
        }
        Intern::new(parts.join("."))
    };
    DefId(fq_name)
}
