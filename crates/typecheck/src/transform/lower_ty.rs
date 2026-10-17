//! Type-resolution helpers used during expression lowering.
//!
//! Provides functions for resolving expression types, binding explicit
//! type annotations, converting literals to types, extracting element
//! types, annotating literal union values, resolving record field
//! indices, converting [`ConstValue`]s to [`Literal`]s, and resolving
//! function-call targets.

use crate::tag_declarations::TagDeclarations;
use diagnostic::Diagnostic;
use diagnostic::code::{BindingCode, PropositionCode};
use internment::Intern;
use std::collections::HashMap;

use crate::analysis::{TyInfer, TyInferEnv, TypeEnv, expr_is_type_surface};
use crate::normal_expr::{SizeCallName, size_expr_to_normal};
use ast::path::ModPath;
use ast::prelude::*;
use ast::{BinderId, ConstValue, HashFloat, NormalExpr, Parameters, TypeReference};

use crate::ty::{Ty, reference_pointee_for_type};
use crate::typed::{DefId, ExprId, TypedExprKind, TypedFileAst, VariantMap};

use super::lower_exprs::LocalVarTypes;
use super::lower_tag::resolve_tag_call_type;

pub(crate) fn resolve_type_reference(
    reference: &TypeReference,
    tag_types: &HashMap<Intern<String>, Ty>,
    local_types: &LocalVarTypes,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
    registry: &crate::TypeRegistry,
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
                registry,
            )),
        },
        TypeReference::Application { name, arguments } => {
            let mut resolved = tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name));
            let parameter_names = tag_params
                .and_then(|all| all.get(name))
                .map(|parameters| parameters.keys().copied().collect::<Vec<_>>())
                .or_else(|| {
                    registry
                        .declaration_for_type(&resolved)
                        .map(|declaration| declaration.parameters.clone())
                })
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
                            registry,
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

/// Resolution context threaded through [`resolve_expr_type`]'s recursive calls.
///
/// `Copy` so the body can destructure it back into the original parameter names.
#[derive(Clone, Copy)]
pub(crate) struct ExprTypeResolverCtx<'a> {
    pub(crate) tag_types: &'a HashMap<Intern<String>, Ty>,
    pub(crate) variant_map: &'a VariantMap,
    pub(crate) receiver_type: Option<&'a Ty>,
    pub(crate) local_var_types: &'a LocalVarTypes,
    pub(crate) fn_return_types: &'a HashMap<DefId, Ty>,
    pub(crate) type_registry: &'a crate::TypeRegistry,
}

/// Resolve the [`Ty`] of a typed parse expression by walking its AST and
/// applying type inference / tag resolution.
pub(crate) fn resolve_expr_type(
    expr: &Typed<Expr>,
    ctx: &ExprTypeResolverCtx<'_>,
    expected_ty: Option<&Ty>,
) -> Ty {
    let ExprTypeResolverCtx {
        tag_types,
        variant_map,
        receiver_type,
        local_var_types,
        fn_return_types,
        type_registry,
    } = *ctx;
    if let Some(resolved) = expr.ty.resolved_ty()
        && !(expected_ty.is_some() && matches!(expr.value, Expr::TagCall(_)))
    {
        return resolved.clone();
    }
    match &expr.value {
        Expr::Return(_) => Ty::Unit,
        Expr::Block(items) => items
            .last()
            .map_or(Ty::Unit, |item| resolve_expr_type(item, ctx, expected_ty)),
        Expr::IntegerDomain(_) => TypeEnv::new(tag_types)
            .with_registry(type_registry)
            .resolve_expr(&expr.value),
        Expr::Emit { code, .. } => {
            let ty = resolve_expr_type(code, ctx, None);
            crate::emission::signature(&ty, type_registry)
                .map(|(_, result)| result)
                .unwrap_or_else(|| expected_ty.cloned().unwrap_or(Ty::Unit))
        }
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
            if fn_call.args.is_none()
                && fn_call.path.segments.is_empty()
                && let Some(ty) = local_var_types.get(&fn_call.path.root)
            {
                return ty.clone();
            }
            let mut target = fn_call
                .definition
                .unwrap_or_else(|| resolve_fn_call_target(&fn_call.path.value, tag_types));
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
                target = DefId::named(Intern::new(parts.join(".")));
            }
            if let Some(return_type) = fn_return_types.get(&target) {
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
            if let Some(path) = &tag_call.qual_path {
                let mut owner = path.value.clone();
                owner.segments.pop();
                let owner = Intern::new(owner.to_qualified_string());
                if let Some(ty) = tag_types.get(&owner)
                    && variant_map.get(&tag_call.name).is_some_and(|variants| {
                        variants.iter().any(|(union, _, _)| *union == owner)
                    })
                {
                    if let Some(expected) = expected_ty
                        && let Some(declaration) = ty.type_id()
                        && expected.type_id() == Some(declaration)
                    {
                        return expected.clone();
                    }
                    return ty.clone();
                }
            }
            resolve_tag_call_type(
                &tag_call.name,
                tag_types,
                variant_map,
                expected_ty,
                type_registry,
            )
        }
        Expr::AnonymousTag(name) if name.as_str() == "Self" => {
            receiver_type.cloned().unwrap_or(Ty::Opaque(*name))
        }
        Expr::AnonymousTag(name) => {
            resolve_tag_call_type(name, tag_types, variant_map, expected_ty, type_registry)
        }
        Expr::Bind(local_bind) => {
            if let Some(ty) = bind_explicit_ty(local_bind, tag_types) {
                return ty;
            }

            match &local_bind.value {
                BindValue::Expr(e) => resolve_expr_type(e, ctx, expected_ty),
                BindValue::Body { exprs: _, ret } => {
                    if !ret.values.is_empty() {
                        let types = ret
                            .values
                            .iter()
                            .map(|typed_expr| resolve_expr_type(typed_expr, ctx, expected_ty))
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
                            Some(typed_expr) => resolve_expr_type(typed_expr, ctx, expected_ty),
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
                WhenArm::Cond { body, .. } | WhenArm::Is { body, .. } => {
                    resolve_expr_type(body, ctx, expected_ty)
                }
                WhenArm::Else(body, _) => resolve_expr_type(body, ctx, expected_ty),
            })
            .unwrap_or_else(|| {
                expected_ty
                    .cloned()
                    .unwrap_or(Ty::Opaque(Intern::new("infer".to_string())))
            }),
        Expr::If(_) => Ty::Unit,
        Expr::Loop(_) => Ty::Unit,
        Expr::SelfRef => local_var_types
            .get(&Intern::from_ref("self"))
            .or(receiver_type)
            .cloned()
            .unwrap_or_else(|| Ty::Opaque(Intern::new("self".to_string()))),
        Expr::FormatString(_) => Ty::Unit,
        Expr::TypeValue(ty) => (**ty).clone(),
        Expr::DomainValue(domain) => Ty::Literal(ast::ConstValue::Domain(domain.clone())),
        Expr::DomainOf(_) => Ty::Opaque(Intern::from_ref("domain")),
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
            let elem_ty = resolve_expr_type(init, ctx, None);
            let elem_ty = if matches!(
                elem_ty,
                Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer)
            ) {
                Ty::i64()
            } else {
                elem_ty
            };
            let size =
                size_expr_to_normal(size, SizeCallName::PathRoot, false).unwrap_or_else(|| {
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
        Expr::AggregateGet { base, .. } => {
            extract_element_type(&resolve_expr_type(base, ctx, None), Some(type_registry))
        }
        Expr::Destructure { value, .. } => resolve_expr_type(value, ctx, expected_ty),
        Expr::RecordSet { value, .. } => resolve_expr_type(value, ctx, expected_ty),
        Expr::RecordGet { base, field } => {
            let base_ty = resolve_expr_type(base, ctx, None);
            let base_definition =
                type_registry.resolved_definition_for_type(reference_referent_or_named(&base_ty));
            base_definition
                .named_field(*field)
                .map(|(_, ty)| ty)
                .unwrap_or_else(|| Ty::Opaque(*field))
        }
        Expr::AggregateSet { value, .. } => resolve_expr_type(value, ctx, expected_ty),
        Expr::Cast { ty, .. } => {
            resolve_type_reference(ty, tag_types, local_var_types, None, type_registry)
        }
        Expr::Target => Ty::Opaque(Intern::from_ref("Target")),
        Expr::TargetQuery { kind, operand } => Ty::AnonymousInteger {
            validity: ast::integer::IntegerValidity::new(ast::integer::IntegerDomain::symbolic(
                NormalExpr::TargetQuery {
                    kind: *kind,
                    operand: operand.clone(),
                },
            )),
        },
        Expr::SliceGet { buf, length, .. } => {
            let source = resolve_expr_type(buf, ctx, None);
            Ty::Array {
                elem: Box::new(extract_element_type(&source, Some(type_registry))),
                size: size_expr_to_normal(&length.value, SizeCallName::PathRoot, false)
                    .unwrap_or_else(|| {
                        NormalExpr::Var(Intern::from_ref("unresolved-slice-length"))
                    }),
            }
        }
        Expr::BufGet { buf, .. } => extract_element_type(
            &resolve_expr_type(buf, ctx, expected_ty),
            Some(type_registry),
        ),
        Expr::BufSet { value, .. } => resolve_expr_type(value, ctx, expected_ty),
        Expr::TakePtr(inner) => {
            let pointee = resolve_expr_type(inner, ctx, None);
            expected_ty
                .map(|ty| type_registry.resolved_definition_for_type(ty))
                .filter(|ty| ty.address_space().is_some())
                .unwrap_or(Ty::Address {
                    pointee: Box::new(pointee),
                    address_space: 0,
                })
        }
        Expr::Ref { inner, mutable, .. } => Ty::Ref {
            inner: Box::new(resolve_expr_type(inner, ctx, expected_ty)),
            mutable: *mutable,
        },
        Expr::Deref(inner) => {
            let inner_ty = resolve_expr_type(inner, ctx, expected_ty);
            reference_pointee_for_type(&inner_ty, Some(type_registry))
                .or_else(|| {
                    type_registry
                        .resolved_definition_for_type(&inner_ty)
                        .pointee_ty()
                        .cloned()
                })
                .unwrap_or(Ty::Unit)
        }
        Expr::Negate(inner) => resolve_expr_type(inner, ctx, expected_ty),
        Expr::RecordLit(fields) => Ty::Record {
            name: Intern::from_ref("Record"),
            fields: fields
                .iter()
                .map(|(name, item)| (*name, Box::new(resolve_expr_type(item, ctx, None))))
                .collect(),
            resolved_params: None,
        },
        Expr::Import(_) => Ty::Unit,
        Expr::Invalid(_) => Ty::Opaque(Intern::from_ref("invalid-expression")),
        Expr::Todo => expected_ty
            .cloned()
            .unwrap_or_else(|| Ty::Opaque(Intern::from_ref("#todo"))),
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
                .map(|item| resolve_expr_type(item, ctx, None))
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
                        let ty = resolve_expr_type(&item.expr, ctx, None);
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
                let resolved = resolve_expr_type(seed, ctx, None);
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
                size: size_expr_to_normal(count, SizeCallName::PathRoot, false).unwrap_or_else(
                    || {
                        NormalExpr::Var(Intern::new(format!(
                            "_unsupported_sequence_size_{:?}",
                            count.span_id
                        )))
                    },
                ),
            }
        }
        Expr::List(items) => Ty::UnresolvedSequence {
            element: items
                .first()
                .map(|item| Box::new(resolve_expr_type(item, ctx, None))),
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
    registry: &crate::TypeRegistry,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    tag_decls: TagDeclarations<'_>,
    binder: BinderId,
    initializer_ty: Option<&Ty>,
) -> Result<Option<Ty>, Diagnostic> {
    if let Some(ty) = bind_refinement_ty(bind) {
        return Ok(Some(ty));
    }
    let applied_surface = bind.type_annotation.as_ref().map(|(name, args)| {
        Expr::TagCall(ast::expr::TagCall {
            name: *name,
            qual_path: bind.type_annotation_qual.clone(),
            args: args.clone(),
        })
    });
    let surface = bind
        .return_tag
        .as_ref()
        .map(|sp| &sp.value)
        .filter(|surface| expr_is_type_surface(surface))
        .or(applied_surface.as_ref());
    let Some(surface) = surface else {
        return Ok(bind_explicit_ty_in_scope(
            bind, registry, tag_types, tag_params, tag_decls,
        ));
    };
    let resolved = TypeEnv::new(tag_types)
        .with_registry(registry)
        .with_tag_params(tag_params)
        .with_tag_declaration_lookup(tag_decls)
        .with_dependent_binder(binder)
        .resolve(surface);
    let resolved = nominalize_explicit_ty_surface(surface, resolved);

    if let Some(initializer) = initializer_ty
        && resolved
            .named_instance_stripping_reference_wrappers()
            .is_some()
        && initializer
            .named_instance_stripping_reference_wrappers()
            .is_some()
    {
        if matches!(surface, Expr::AnonymousTag(_))
            && let (
                Ty::Named {
                    instance: expected,
                    name,
                },
                Ty::Named {
                    instance: actual, ..
                },
            ) = (&resolved, initializer)
            && expected.declaration == actual.declaration
            && tag_params.get(name).is_some_and(|parameters| {
                parameters
                    .values()
                    .all(|parameter| !matches!(parameter.kind, ast::ParameterKind::Default(_)))
            })
        {
            return Ok(Some(initializer.clone()));
        }
        let substitution = crate::analysis::unify_type_args_with_registry(
            &[ast::ty::TyArg::Type(Box::new(
                resolved.without_reference_wrappers().clone(),
            ))],
            &[ast::ty::TyArg::Type(Box::new(
                initializer.without_reference_wrappers().clone(),
            ))],
            Some(registry),
        )
        // type-binding-mismatch
        .map_err(|message| Diagnostic::new(BindingCode::Mismatch, message))?;
        return Ok(Some(substitution.apply_to_ty(&resolved)));
    }

    Ok(match (initializer_ty, &resolved) {
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
    })
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
    registry: &crate::TypeRegistry,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: &HashMap<Intern<String>, Parameters>,
    tag_decls: TagDeclarations<'_>,
) -> Option<Ty> {
    if let Some(ty) = bind_refinement_ty(bind) {
        return Some(ty);
    }
    if !bind.named_results.is_empty() {
        let env = TypeEnv::new(tag_types)
            .with_registry(registry)
            .with_tag_params(tag_params)
            .with_tag_declaration_lookup(tag_decls);
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
            .with_registry(registry)
            .with_tag_params(tag_params)
            .with_tag_declaration_lookup(tag_decls)
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
            .with_registry(registry)
            .with_tag_params(tag_params)
            .with_tag_declaration_lookup(tag_decls)
            .resolve(&annotated);
        let resolved = nominalize_explicit_ty_surface(&annotated, ty);
        return Some(resolved);
    }
    bind_explicit_ty(bind, tag_types)
}

pub(super) fn validate_domain_annotation(
    typed: &mut TypedFileAst,
    bind: &Bind,
    value: ExprId,
    expected: &Ty,
    diagnostic_expression: Option<ExprId>,
) {
    if matches!(expected, Ty::Proposition(_))
        || typed
            .exprs
            .ty_of(value)
            .is_some_and(|ty| matches!(ty, Ty::Proposition(_)))
    {
        if typed.exprs.ty_of(value) != Some(expected) {
            typed.declaration_flaws.push((
                bind.name_span,
                // type-proposition-mismatch
                diagnostic::Diagnostic::new(
                    PropositionCode::Mismatch,
                    "the comparison does not match the declared proposition",
                )
                .at_span_id(bind.name_span, &typed.span_table),
            ));
        }
        return;
    }
    if bind.return_refinement.is_none()
        && !bind
            .return_tag
            .as_ref()
            .is_some_and(|surface| matches!(surface.value, Expr::IntegerDomain(_)))
    {
        return;
    }
    let Some(expected) = expected.anonymous_integer_validity() else {
        return;
    };
    let expected = expected.clone();
    validate_integer_validity(typed, bind, value, &expected, diagnostic_expression);
}

pub(super) fn validate_projected_return(
    typed: &mut TypedFileAst,
    bind: &Bind,
    value: ExprId,
    expected: &Ty,
) {
    if !typed
        .exprs
        .ty_of(value)
        .is_some_and(|ty| ty.anonymous_integer_validity().is_some())
    {
        return;
    }
    let Some(validity) = typed
        .type_registry
        .integer_validity_for_type(expected)
        .cloned()
    else {
        return;
    };
    validate_integer_validity(typed, bind, value, &validity, None);
    typed.exprs.ty[value.as_usize()] = expected.clone();
}

fn validate_integer_validity(
    typed: &mut TypedFileAst,
    bind: &Bind,
    value: ExprId,
    expected: &ast::integer::IntegerValidity,
    diagnostic_expression: Option<ExprId>,
) {
    if let Some(crate::typed::TypedExprKind::When(expression)) = typed.exprs.kind_of(value) {
        let bodies = expression
            .arms
            .iter()
            .map(|arm| match arm {
                crate::typed::TypedWhenArm::Cond { body, .. }
                | crate::typed::TypedWhenArm::Is { body, .. }
                | crate::typed::TypedWhenArm::Else(body, _) => *body,
            })
            .collect::<Vec<_>>();
        for body in bodies {
            if !crate::analysis::expression_returns(typed.resolved_program(), body) {
                validate_integer_validity(typed, bind, body, expected, diagnostic_expression);
            }
        }
        return;
    }
    let actual = typed
        .exprs
        .integer_knowledge_of(value)
        .and_then(Option::as_ref)
        .and_then(super::place::knowledge_domain)
        .or_else(|| {
            typed
                .exprs
                .ty_of(value)
                .and_then(|ty| {
                    typed
                        .type_registry
                        .integer_validity_for_type(ty.without_reference_wrappers())
                })
                .map(|validity| validity.domain().clone())
        });
    let proof = actual
        .as_ref()
        .map_or(ast::integer::ContainmentProof::Unknown, |actual| {
            actual.prove_containment(expected.domain())
        });
    if let Some(code) = proof.diagnostic_code()
        && !(proof == ast::integer::ContainmentProof::Unknown
            && (typed.type_registry.is_target_dependent_validity(expected)
                || (typed.target_layout.is_none()
                    && typed
                        .exprs
                        .ty_of(value)
                        .is_some_and(|ty| typed.type_registry.is_target_dependent_integer(ty)))))
    {
        let diagnostic = diagnostic::Diagnostic::new(
            code,
            "integer domain annotation requires the value to satisfy the target validity",
        )
        .at_span_id(bind.name_span, &typed.span_table);
        if let Some(expression) = diagnostic_expression {
            typed.exprs.facts.semantic_facts.flaws[expression.index()].push(diagnostic);
        } else {
            typed.declaration_flaws.push((bind.name_span, diagnostic));
        }
    }
}

fn bind_refinement_ty(bind: &Bind) -> Option<Ty> {
    let predicate = bind.return_refinement.as_ref()?;
    if bind.return_tag.is_none()
        && let ast::PredicateExpr::Proposition(proposition) = predicate
    {
        return Some(Ty::Proposition(proposition.clone()));
    }
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
    DefId::named(fq_name)
}
