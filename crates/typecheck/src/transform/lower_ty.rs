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
use ast::{BinderId, ConstValue, HashFloat, NormalExpr, Parameters, TypeReference};

use crate::ty::Ty;
use crate::typed::{DefId, ExprId, TypedExprKind, TypedFileAst, VariantMap};

use super::lower_exprs::LocalVarTypes;
use super::lower_tag::resolve_tag_call_type;

fn tuple_alloc_size_expr(size: &Expr) -> Option<NormalExpr> {
    match size {
        Expr::Lit(ast::Literal::Int(n)) => Some(NormalExpr::from(*n)),
        Expr::Lit(ast::Literal::Number(n)) => Some(NormalExpr::from(*n as i128)),
        Expr::FnCall(call) if call.args.is_none() => Some(NormalExpr::Var(call.path.value.root)),
        Expr::Bind(bind) => Some(NormalExpr::Var(bind.name)),
        Expr::Binary(binary) => {
            let lhs = tuple_alloc_size_expr(&binary.lhs.value)?;
            let rhs = tuple_alloc_size_expr(&binary.rhs.value)?;
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

pub(crate) fn resolve_type_reference(
    reference: &TypeReference,
    tag_types: &HashMap<Intern<String>, Ty>,
    local_types: &LocalVarTypes,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> Ty {
    match reference {
        TypeReference::Unit => Ty::Unit,
        TypeReference::Nominal(name) => local_types
            .get(name)
            .or_else(|| tag_types.get(name))
            .cloned()
            .unwrap_or(Ty::Opaque(*name)),
        TypeReference::Pointer(inner) => Ty::Ptr {
            inner: Box::new(resolve_type_reference(
                inner,
                tag_types,
                local_types,
                tag_params,
            )),
        },
        TypeReference::Application { name, arguments } => {
            let mut resolved = tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name));
            let parameter_names = tag_params
                .and_then(|all| all.get(name))
                .map(|parameters| parameters.keys().copied().collect::<Vec<_>>())
                .unwrap_or_default();
            let arguments = arguments
                .iter()
                .enumerate()
                .map(|(index, argument)| {
                    let parameter = parameter_names
                        .get(index)
                        .copied()
                        .unwrap_or_else(|| Intern::new(format!("argument{index}")));
                    (
                        parameter,
                        ast::TyArg::Type(Box::new(resolve_type_reference(
                            argument,
                            tag_types,
                            local_types,
                            tag_params,
                        ))),
                    )
                })
                .collect::<Vec<_>>();
            match &mut resolved {
                Ty::Named { instance, .. } => {
                    instance.arguments = arguments.clone();
                }
                Ty::Record {
                    resolved_params, ..
                }
                | Ty::Union {
                    resolved_params, ..
                } => *resolved_params = Some(arguments),
                _ => {}
            }
            resolved
        }
    }
}

/// Resolve the [`Ty`] of a typed parse expression by walking its AST and
/// applying type inference / tag resolution.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_expr_type(
    expr: &Typed<Expr>,
    tag_types: &HashMap<Intern<String>, Ty>,
    variant_map: &VariantMap,
    receiver_type: Option<&Ty>,
    expected_ty: Option<&Ty>,
    local_var_types: &LocalVarTypes,
    fn_return_types: &HashMap<DefId, Ty>,
    type_registry: &crate::TypeRegistry,
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
            let mut target = resolve_fn_call_target(&fn_call.path.value, tag_types);
            if fn_call.path.value.root.as_str() == "Self"
                && let Some(receiver) = receiver_type.and_then(Ty::type_name)
            {
                let mut parts = vec![receiver.as_str()];
                parts.extend(
                    fn_call
                        .path
                        .value
                        .segments
                        .iter()
                        .map(|segment| segment.as_str()),
                );
                target = DefId(Intern::new(parts.join(".")));
            }
            let overload = DefId(Intern::new(format!(
                "{}$arity{}",
                target.0,
                fn_call.args.as_ref().map_or(0, Vec::len)
            )));
            if let Some(return_type) = fn_return_types
                .get(&target)
                .or_else(|| fn_return_types.get(&overload))
            {
                return return_type.clone();
            }
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
                    type_registry,
                ),
                BindValue::Body { exprs: _, ret } => match &ret.value {
                    Some(typed_expr) => resolve_expr_type(
                        typed_expr,
                        tag_types,
                        variant_map,
                        receiver_type,
                        expected_ty,
                        local_var_types,
                        fn_return_types,
                        type_registry,
                    ),
                    None => Ty::Unit,
                },
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
                    type_registry,
                ),
                WhenArm::Else(body, _) => resolve_expr_type(
                    body,
                    tag_types,
                    variant_map,
                    receiver_type,
                    expected_ty,
                    local_var_types,
                    fn_return_types,
                    type_registry,
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
                    type_registry,
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
            let expected_ty = expected_ty.map(Ty::without_reference_wrappers);
            match expected_ty {
                Some(expected_ty)
                    if matches!(expected_ty, Ty::Named { .. } | Ty::Array { .. })
                        || matches!(
                            &expected_ty,
                            Ty::Opaque(name) if name.as_str() == "unresolved-array-construction"
                        ) =>
                {
                    return expected_ty.clone();
                }
                _ => {}
            }
            let elem_ty = resolve_expr_type(
                init,
                tag_types,
                variant_map,
                receiver_type,
                None,
                local_var_types,
                fn_return_types,
                type_registry,
            );
            let elem_ty = if matches!(
                elem_ty,
                Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)
            ) {
                Ty::i64()
            } else {
                elem_ty
            };
            let size = tuple_alloc_size_expr(size).unwrap_or_else(|| {
                NormalExpr::Var(Intern::new(format!(
                    "_unsupported_tuple_size_{:?}",
                    size.span_id
                )))
            });
            Ty::Array {
                elem: Box::new(elem_ty),
                size,
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
            type_registry,
        )),
        Expr::Destructure { value, .. } => resolve_expr_type(
            value,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
            type_registry,
        ),
        Expr::RecordSet { value, .. } => resolve_expr_type(
            value,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
            type_registry,
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
                type_registry,
            );
            let base_definition =
                type_registry.resolved_definition_for_type(reference_referent_or_named(&base_ty));
            match &base_definition {
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
            type_registry,
        ),
        Expr::Cast { ty, .. } => resolve_type_reference(ty, tag_types, local_var_types, None),
        Expr::TargetQuery { kind, operand } => Ty::AnonymousInteger {
            validity: ast::integer::IntegerValidity::new(ast::integer::IntegerDomain::symbolic(
                NormalExpr::TargetQuery {
                    kind: *kind,
                    operand: operand.clone(),
                },
            )),
        },
        Expr::BufGet { buf, .. } => extract_element_type(&resolve_expr_type(
            buf,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
            type_registry,
        )),
        Expr::BufSet { value, .. } => resolve_expr_type(
            value,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
            type_registry,
        ),
        Expr::TakePtr(inner) => {
            let pointee = resolve_expr_type(
                inner,
                tag_types,
                variant_map,
                receiver_type,
                None,
                local_var_types,
                fn_return_types,
                type_registry,
            );
            expected_ty
                .filter(|ty| ty.address_space().is_some())
                .cloned()
                .unwrap_or(Ty::Address {
                    pointee: Box::new(pointee),
                    address_space: 0,
                })
        }
        Expr::Ref { inner, mutable, .. } => Ty::Ref {
            inner: Box::new(resolve_expr_type(
                inner,
                tag_types,
                variant_map,
                receiver_type,
                expected_ty,
                local_var_types,
                fn_return_types,
                type_registry,
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
            type_registry,
        ),
        Expr::ConsumeArg(inner) => resolve_expr_type(
            inner,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
            type_registry,
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
                type_registry,
            );
            inner_ty.pointee_ty().cloned().unwrap_or(Ty::Unit)
        }
        Expr::Negate(inner) => resolve_expr_type(
            inner,
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            local_var_types,
            fn_return_types,
            type_registry,
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
                        type_registry,
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
                        type_registry,
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
                    type_registry,
                )
            })
            .unwrap_or(Ty::Opaque(Intern::new("List".to_string()))),
    }
}

/// When a bind is annotated with a const union, give its literal the union type rather than `String`.
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
    if let Some(ty) = bind_refinement_ty(bind) {
        return Some(ty);
    }
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

    if let (Some(initializer), Some(initializer_instance), Some(resolved_instance)) = (
        initializer_ty,
        initializer_ty.and_then(Ty::named_instance),
        resolved.named_instance(),
    ) && initializer_instance.declaration == resolved_instance.declaration
    {
        return Some(initializer.clone());
    }

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
    if let Some(ty) = bind_refinement_ty(bind) {
        return Some(ty);
    }
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
            .map(|ty| nominalize_explicit_ty_surface(&ast::Expr::AnonymousTag(*name), ty))
            .or(Some(Ty::Opaque(*name)));
    }
    if let Some((name, args)) = &bind.type_annotation {
        let annotated = ast::Expr::TagCall(ast::expr::TagCall {
            name: *name,
            qual_path: None,
            args: args.clone(),
        });
        return Some(nominalize_explicit_ty_surface(
            &annotated,
            TypeEnv::new(tag_types).resolve(&annotated),
        ));
    }
    None
}

pub(crate) fn bind_explicit_ty_in_scope(
    bind: &Bind,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    tag_decls: &ast::TagMap,
) -> Option<Ty> {
    if let Some(ty) = bind_refinement_ty(bind) {
        return Some(ty);
    }
    if let Some(sp) = &bind.return_tag
        && expr_is_type_surface(&sp.value)
    {
        let ty = TypeEnv::new(tag_types)
            .with_tag_params(tag_params)
            .with_tag_decls(tag_decls)
            .resolve(&sp.value);
        return Some(nominalize_explicit_ty_surface(&sp.value, ty));
    }
    if let Some((name, args)) = &bind.type_annotation {
        let annotated = ast::Expr::TagCall(ast::expr::TagCall {
            name: *name,
            qual_path: None,
            args: args.clone(),
        });
        let ty = TypeEnv::new(tag_types)
            .with_tag_params(tag_params)
            .with_tag_decls(tag_decls)
            .resolve(&annotated);
        let resolved = nominalize_explicit_ty_surface(&annotated, ty);
        return Some(resolved);
    }
    bind_explicit_ty(bind, tag_types)
}

fn bind_refinement_ty(bind: &Bind) -> Option<Ty> {
    let predicate = bind.return_refinement.as_ref()?;
    Some(Ty::AnonymousInteger {
        validity: ast::integer::IntegerValidity::new(
            ast::integer::IntegerDomain::from_named_refinement(predicate, bind.name),
        ),
    })
}

pub(crate) fn materialize_raw_pointer(
    default: &Ty,
    pointee: &Ty,
    registry: &crate::TypeRegistry,
) -> Option<Ty> {
    let Ty::Named { instance, name } = default else {
        return None;
    };
    let definition = registry.resolved_definition_for_type(default);
    let pointee_param = match &definition {
        Ty::Ptr { inner } => inner,
        Ty::Address { pointee: inner, .. } => inner,
        _ => return None,
    };
    let Ty::Opaque(parameter) = pointee_param.as_ref() else {
        return None;
    };
    let mut instance = instance.clone();
    instance.arguments = vec![(*parameter, ast::TyArg::Type(Box::new(pointee.clone())))];
    Some(Ty::Named {
        instance,
        name: *name,
    })
}

pub(crate) fn materialize_fixed_array(
    default: &Ty,
    contract: &ast::FixedArrayContract,
    element: &Ty,
    length: NormalExpr,
) -> Option<Ty> {
    let Ty::Named { instance, name } = default else {
        return None;
    };
    let mut instance = instance.clone();
    instance.arguments = vec![
        (
            contract.element_parameter,
            ast::TyArg::Type(Box::new(element.clone())),
        ),
        (contract.length_parameter, ast::TyArg::Const(length)),
    ];
    Some(Ty::Named {
        instance,
        name: *name,
    })
}

fn reference_referent_or_named(ty: &Ty) -> &Ty {
    match ty {
        Ty::Ref { inner, .. } => reference_referent_or_named(inner),
        _ => ty,
    }
}

/// Convert a parsed [`Literal`] to its corresponding [`Ty`].
pub(crate) fn lit_to_ty(lit: &Literal) -> Ty {
    match lit {
        Literal::Number(_) => Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer),
        Literal::Float(HashFloat(f)) => Ty::Float {
            value: Some(HashFloat(*f)),
        },
        Literal::Int(_) => Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer),
        Literal::String(s) => Ty::Literal(ConstValue::String(s.clone())),
    }
}

/// Extract the element type from an array or tuple type.
pub(crate) fn extract_element_type(ty: &Ty) -> Ty {
    match ty {
        Ty::Array { elem, .. } => *elem.clone(),
        Ty::Named { instance, .. } => instance
            .arguments
            .iter()
            .find_map(|(_, argument)| match argument {
                ast::TyArg::Type(element) => Some((**element).clone()),
                ast::TyArg::Const(_) => None,
            })
            .unwrap_or_else(|| Ty::Opaque(Intern::from_ref("unresolved-array-element"))),
        Ty::Tuple(fields) => fields.first().cloned().unwrap_or(Ty::Unit),
        ty if ty.pointee_ty().is_some() => ty.pointee_ty().cloned().unwrap_or(Ty::Unit),
        _ => Ty::Opaque(Intern::new("infer".to_string())),
    }
}

/// Resolve a function-call path to a [`DefId`].
pub(crate) fn resolve_fn_call_target(
    path: &ModPath,
    tag_types: &HashMap<Intern<String>, Ty>,
) -> DefId {
    let root = if path.root.as_str() == "Self" {
        tag_types
            .get(&path.root)
            .and_then(Ty::type_name)
            .copied()
            .unwrap_or(path.root)
    } else {
        path.root
    };
    let fq_name = if path.segments.is_empty() {
        root
    } else {
        let mut parts: Vec<&str> = vec![root.as_str()];
        for seg in &path.segments {
            parts.push(seg.as_str());
        }
        Intern::new(parts.join("."))
    };
    DefId(fq_name)
}
