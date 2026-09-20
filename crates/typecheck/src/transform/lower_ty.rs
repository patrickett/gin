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

use crate::ty::{Ty, reference_pointee_for_type};
use crate::typed::{DefId, ExprId, TypedExprKind, TypedFileAst, VariantMap};

use super::lower_exprs::LocalVarTypes;
use super::lower_tag::resolve_tag_call_type;

fn fixed_array_size_expr(size: &Expr) -> Option<NormalExpr> {
    match size {
        Expr::Lit(ast::Literal::Int(n)) => Some(NormalExpr::from(*n)),
        Expr::Lit(ast::Literal::Number(n)) => Some(NormalExpr::from(*n as i128)),
        Expr::FnCall(call) if call.args.is_none() => Some(NormalExpr::Var(call.path.value.root)),
        Expr::Bind(bind) => Some(NormalExpr::Var(bind.name)),
        Expr::Binary(binary) => {
            let lhs = fixed_array_size_expr(&binary.lhs.value)?;
            let rhs = fixed_array_size_expr(&binary.rhs.value)?;
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
        TypeReference::Nominal(name) => match name.as_str() {
            _ => local_types
                .get(name)
                .or_else(|| tag_types.get(name))
                .cloned()
                .unwrap_or(Ty::Opaque(*name)),
        },
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
        Expr::Asm { .. } => expected_ty.cloned().unwrap_or(Ty::Unit),
        Expr::Register(inner) => inner
            .resolved_ty()
            .cloned()
            .unwrap_or_else(|| expected_ty.cloned().unwrap_or(Ty::Unit)),
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
                BindValue::Body { exprs: _, ret } => {
                    if !ret.values.is_empty() {
                        let types = ret
                            .values
                            .iter()
                            .map(|typed_expr| {
                                resolve_expr_type(
                                    typed_expr,
                                    tag_types,
                                    variant_map,
                                    receiver_type,
                                    expected_ty,
                                    local_var_types,
                                    fn_return_types,
                                    type_registry,
                                )
                            })
                            .collect::<Vec<_>>();
                        if local_bind.named_results.len() == types.len() && types.len() > 1 {
                            Ty::Sequence(
                                local_bind
                                    .named_results
                                    .iter()
                                    .zip(types)
                                    .map(|(result, ty)| ast::ty::SequenceTypeElement {
                                        name: result.name,
                                        ty: Box::new(ty),
                                    })
                                    .collect(),
                            )
                        } else if let [ty] = types.as_slice() {
                            ty.clone()
                        } else {
                            Ty::sequence(
                                types
                                    .into_iter()
                                    .map(|ty| ast::ty::SequenceTypeElement {
                                        name: None,
                                        ty: Box::new(ty),
                                    })
                                    .collect(),
                            )
                        }
                    } else {
                        match ret.values.first() {
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
                        }
                    }
                }
                BindValue::Unassigned => Ty::Unit,
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
        Expr::FormatString(_) => Ty::Unit,
        Expr::Range(_) => Ty::Opaque(Intern::new("range_expr".to_string())),
        Expr::FixedArrayAlloc { init, size } => {
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
            let size = fixed_array_size_expr(size).unwrap_or_else(|| {
                NormalExpr::Var(Intern::new(format!(
                    "_unsupported_fixed_array_size_{:?}",
                    size.span_id
                )))
            });
            Ty::Array {
                elem: Box::new(elem_ty),
                size,
            }
        }
        Expr::AggregateGet { base, .. } => extract_element_type(
            &resolve_expr_type(
                base,
                tag_types,
                variant_map,
                receiver_type,
                None,
                local_var_types,
                fn_return_types,
                type_registry,
            ),
            Some(type_registry),
        ),
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
            base_definition
                .named_field(*field)
                .map(|(_, ty)| ty)
                .unwrap_or_else(|| Ty::Opaque(*field))
        }
        Expr::AggregateSet { value, .. } => resolve_expr_type(
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
        Expr::Target => Ty::Opaque(Intern::from_ref("Target")),
        Expr::TargetQuery { kind, operand } => Ty::AnonymousInteger {
            validity: ast::integer::IntegerValidity::new(ast::integer::IntegerDomain::symbolic(
                NormalExpr::TargetQuery {
                    kind: *kind,
                    operand: operand.clone(),
                },
            )),
        },
        Expr::BufGet { buf, .. } => extract_element_type(
            &resolve_expr_type(
                buf,
                tag_types,
                variant_map,
                receiver_type,
                expected_ty,
                local_var_types,
                fn_return_types,
                type_registry,
            ),
            Some(type_registry),
        ),
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
                .map(|ty| type_registry.resolved_definition_for_type(ty))
                .filter(|ty| ty.address_space().is_some())
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
            reference_pointee_for_type(&inner_ty, Some(type_registry))
                .or_else(|| {
                    type_registry
                        .resolved_definition_for_type(&inner_ty)
                        .pointee_ty()
                        .cloned()
                })
                .unwrap_or(Ty::Unit)
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
        Expr::RecordLit(fields) => Ty::Record {
            name: Intern::from_ref("Record"),
            fields: fields
                .iter()
                .map(|(name, item)| {
                    (
                        *name,
                        Box::new(resolve_expr_type(
                            item,
                            tag_types,
                            variant_map,
                            receiver_type,
                            None,
                            local_var_types,
                            fn_return_types,
                            type_registry,
                        )),
                    )
                })
                .collect(),
            resolved_params: None,
        },
        Expr::Invalid(_) => Ty::Opaque(Intern::from_ref("invalid-expression")),
        Expr::ResultValues(items) if items.is_empty() => Ty::Unit,
        Expr::ResultValues(_) => Ty::Opaque(Intern::from_ref("result-values")),
        Expr::ArrayLit(items) => {
            let expected_ty = expected_ty.map(Ty::without_reference_wrappers);
            if let Some(expected) = expected_ty
                && (matches!(expected, Ty::Array { .. })
                    || crate::representation::Repr::array_parts(expected, Some(type_registry))
                        .is_some())
            {
                return expected.clone();
            }
            let elem_ty = items
                .first()
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
                .unwrap_or(Ty::Unit);
            let elem_ty = if matches!(
                elem_ty,
                Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)
            ) {
                Ty::i64()
            } else {
                elem_ty
            };
            Ty::Array {
                elem: Box::new(elem_ty),
                size: NormalExpr::from(items.len() as i128),
            }
        }
        Expr::SequenceLiteral(ast::SequenceLiteral::Elements(items)) => {
            if let Some(expected) = expected_ty.map(Ty::without_reference_wrappers)
                && (matches!(
                    expected,
                    Ty::Array { .. } | Ty::Sequence(_) | Ty::Record { .. }
                ) || expected
                    .named_instance()
                    .and_then(|instance| type_registry.declaration_for_instance(instance))
                    .is_some_and(|declaration| declaration.sequence_literal.is_some())
                    || crate::representation::Repr::array_parts(expected, Some(type_registry))
                        .is_some())
            {
                return expected.clone();
            }
            Ty::sequence(
                items
                    .iter()
                    .map(|item| {
                        let ty = resolve_expr_type(
                            &item.expr,
                            tag_types,
                            variant_map,
                            receiver_type,
                            None,
                            local_var_types,
                            fn_return_types,
                            type_registry,
                        );
                        let ty =
                            if matches!(ty, Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)) {
                                Ty::i64()
                            } else {
                                ty
                            };
                        ast::ty::SequenceTypeElement {
                            name: item.name,
                            ty: Box::new(ty),
                        }
                    })
                    .collect(),
            )
        }
        Expr::SequenceLiteral(ast::SequenceLiteral::Repeat { seed, count }) => {
            if let Some(expected) = expected_ty.map(Ty::without_reference_wrappers)
                && (matches!(expected, Ty::Array { .. })
                    || crate::representation::Repr::array_parts(expected, Some(type_registry))
                        .is_some())
            {
                return expected.clone();
            }
            let expected_elem = expected_ty
                .map(Ty::without_reference_wrappers)
                .and_then(|ty| match ty {
                    Ty::Array { elem, .. } => Some((**elem).clone()),
                    _ => None,
                });
            let elem_ty = expected_elem.unwrap_or_else(|| {
                let resolved = resolve_expr_type(
                    seed,
                    tag_types,
                    variant_map,
                    receiver_type,
                    None,
                    local_var_types,
                    fn_return_types,
                    type_registry,
                );
                if matches!(
                    resolved,
                    Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)
                ) {
                    Ty::i64()
                } else {
                    resolved
                }
            });
            Ty::Array {
                elem: Box::new(elem_ty),
                size: fixed_array_size_expr(count).unwrap_or_else(|| {
                    NormalExpr::Var(Intern::new(format!(
                        "_unsupported_sequence_size_{:?}",
                        count.span_id
                    )))
                }),
            }
        }
        Expr::List(items) => Ty::UnresolvedSequence {
            element: items.first().map(|item| {
                Box::new(resolve_expr_type(
                    item,
                    tag_types,
                    variant_map,
                    receiver_type,
                    None,
                    local_var_types,
                    fn_return_types,
                    type_registry,
                ))
            }),
            length: NormalExpr::from(items.len() as i128),
        },
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
    if typed.exprs.facts.const_facts.const_value[idx].is_none()
        && let Some(value) = values.first()
    {
        typed.exprs.facts.const_facts.const_value[idx] = Some(value.clone());
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
        return bind_explicit_ty_in_scope(bind, tag_types, tag_params, tag_decls);
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
    if !bind.named_results.is_empty() {
        let env = TypeEnv::new(tag_types);
        let elements = bind
            .named_results
            .iter()
            .map(|result| ast::ty::SequenceTypeElement {
                name: result.name,
                ty: Box::new(env.resolve_expr(&result.ty.value)),
            })
            .collect::<Vec<_>>();
        return Some(Ty::results(elements));
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
    if !bind.named_results.is_empty() {
        let env = TypeEnv::new(tag_types)
            .with_tag_params(tag_params)
            .with_tag_decls(tag_decls);
        let elements = bind
            .named_results
            .iter()
            .map(|result| ast::ty::SequenceTypeElement {
                name: result.name,
                ty: Box::new(env.resolve_expr(&result.ty.value)),
            })
            .collect::<Vec<_>>();
        return Some(Ty::results(elements));
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
        Literal::String(text) => Ty::Array {
            elem: Box::new(Ty::u8()),
            size: NormalExpr::from(text.len() as i128),
        },
    }
}

/// Extract the element type from an array or value-row type.
pub(crate) fn extract_element_type(ty: &Ty, registry: Option<&crate::TypeRegistry>) -> Ty {
    let base = crate::ty::reference_pointee_for_type(ty, registry).unwrap_or_else(|| ty.clone());
    if let Some((element, _)) = crate::representation::Repr::array_parts(&base, registry) {
        return element;
    }
    if let Ty::Ref { inner, .. } = ty {
        return extract_element_type(inner, registry);
    }
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
        Ty::Record { fields, .. } => fields
            .first()
            .map(|(_, field)| (**field).clone())
            .unwrap_or(Ty::Unit),
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
