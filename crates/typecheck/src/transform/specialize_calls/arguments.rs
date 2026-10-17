use std::collections::BTreeMap;
use std::ops::ControlFlow;

use ast::{Bind, Expr, NormalExpr, Ty, TyArg};
use internment::Intern;

use crate::analysis::{TypeEnv, unify_type_args_with_registry};
use crate::normal_expr::normalize_type_constants;
use crate::typed::{DefId, TypedBind, TypedFileAst};

pub(super) fn infer(
    bind: &TypedBind,
    args: &[crate::ExprId],
    typed: &TypedFileAst,
    call: crate::ExprId,
    owner: Option<&Bind>,
) -> Result<Option<Arguments>, String> {
    for name in bind.array_length_parameters() {
        if typed.fn_return_types.contains_key(&DefId::named(name))
            || typed.defs.contains_key(&DefId::named(name))
        {
            return Err(format!(
                "array length `{name}` already denotes a value and cannot be inferred"
            ));
        }
    }
    let params = bind
        .params
        .iter()
        .zip(&bind.param_kinds)
        .filter(|(_, kind)| !matches!(kind, ast::ParamKind::Type))
        .map(|(param, _)| param)
        .collect::<Vec<_>>();
    if args.len() != params.len() {
        return Err(format!(
            "expected {} arguments, found {}",
            params.len(),
            args.len()
        ));
    }
    let expected = params
        .iter()
        .map(|(_, ty)| TyArg::Type(Box::new(ty.clone())))
        .collect::<Vec<_>>();
    let actual = args
        .iter()
        .zip(&params)
        .map(|(arg, (_, expected))| {
            TyArg::Type(Box::new(crate::staging::normalize_inferred_value_type(
                expected,
                &typed.exprs.ty[arg.as_usize()],
            )))
        })
        .collect::<Vec<_>>();
    let mut subst = unify_type_args_with_registry(&expected, &actual, Some(&typed.type_registry))?;
    let mut explicit_types = BTreeMap::new();
    if let Some(owner) = owner {
        struct TypeArguments<'a> {
            span: usize,
            typed: &'a TypedFileAst,
            template: &'a TypedBind,
            types: &'a mut BTreeMap<Intern<String>, Ty>,
        }
        impl ast::folder::Visitor for TypeArguments<'_> {
            fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
                if let Expr::FnCall(call) = expr
                    && self.typed.span_table.get(call.path.span_id).start() == self.span
                    && let Some(args) = &call.args
                {
                    let Some(order) = argument_order(args, self.template) else {
                        return ControlFlow::Break(());
                    };
                    for (index, argument) in order.into_iter().enumerate() {
                        let (name, _) = &self.template.params[index];
                        let kind = &self.template.param_kinds[index];
                        let arg = match &args[argument].value {
                            Expr::Bind(bind) => match &bind.value {
                                ast::BindValue::Expr(value) => &value.value,
                                _ => continue,
                            },
                            value => value,
                        };
                        if matches!(kind, ast::ParamKind::Type) {
                            let tags = self
                                .typed
                                .tag_types
                                .iter()
                                .map(|(name, ty)| (name.0, ty.clone()))
                                .collect();
                            let parameters = self
                                .typed
                                .eval_ast
                                .tags
                                .iter()
                                .filter_map(|(name, declaration)| {
                                    declaration
                                        .params
                                        .as_ref()
                                        .map(|params| (*name, params.clone()))
                                })
                                .collect();
                            let ty = TypeEnv::new(&tags)
                                .with_tag_params(&parameters)
                                .with_tag_decls(&self.typed.eval_ast.tags)
                                .with_registry(&self.typed.type_registry)
                                .resolve_expr(arg);
                            self.types.insert(*name, ty);
                        }
                    }
                    return ControlFlow::Break(());
                }
                ast::folder::walk_expr(self, expr)
            }
        }
        use ast::folder::Visitor;
        let _ = TypeArguments {
            span: typed
                .span_table
                .get(typed.exprs.span[call.as_usize()])
                .start(),
            typed,
            template: bind,
            types: &mut explicit_types,
        }
        .visit_bind(owner);
    }
    for (name, explicit) in explicit_types {
        if let Some(inferred) = subst.types.get(&name)
            && type_surface(inferred, typed).is_some()
        {
            unify_type_args_with_registry(
                &[TyArg::Type(Box::new(explicit.clone()))],
                &[TyArg::Type(Box::new(inferred.clone()))],
                Some(&typed.type_registry),
            )?;
        }
        subst.types.insert(name, explicit);
    }
    let mut values = BTreeMap::new();
    for name in bind.type_parameters() {
        if let Some(expr) = subst
            .types
            .get(&name)
            .and_then(|ty| type_surface(ty, typed))
        {
            values.insert(name, expr);
        }
        if !values.contains_key(&name) {
            return Ok(None);
        }
    }
    for ((_, expected), actual) in params.iter().zip(args) {
        let actual = &typed.exprs.ty[actual.as_usize()];
        if matches!(expected, Ty::Array { .. })
            && normalize_type_constants(subst.apply_to_ty(expected))
                != normalize_type_constants(actual.clone())
        {
            return Err(format!(
                "array argument does not match {}",
                expected.format_for_hover()
            ));
        }
    }
    for name in bind.array_length_parameters() {
        let size = subst
            .consts
            .get(&name)
            .and_then(NormalExpr::as_const_int)
            .filter(|size| *size >= 0)
            .ok_or_else(|| format!("cannot infer array length `{name}`"))?;
        values.insert(name, Expr::Lit(ast::Literal::Int(size.into())));
    }
    let receivers = subst
        .types
        .iter()
        .filter_map(|(name, ty)| {
            ty.named_instance()
                .map(|instance| (*name, instance.declaration))
        })
        .collect();
    Ok(Some(Arguments { values, receivers }))
}

pub(super) fn for_receiver(
    bind: &TypedBind,
    receiver: &Ty,
    typed: &TypedFileAst,
) -> Option<Arguments> {
    let expected = bind.params.first()?.1.without_reference_wrappers();
    let actual = receiver.without_reference_wrappers();
    let subst = unify_type_args_with_registry(
        &[TyArg::Type(Box::new(expected.clone()))],
        &[TyArg::Type(Box::new(actual.clone()))],
        Some(&typed.type_registry),
    )
    .ok()?;
    let values = bind
        .type_parameters()
        .into_iter()
        .map(|name| Some((name, type_surface(subst.types.get(&name)?, typed)?)))
        .collect::<Option<BTreeMap<_, _>>>()?;
    let receivers = subst
        .types
        .iter()
        .filter_map(|(name, ty)| {
            ty.named_instance()
                .map(|instance| (*name, instance.declaration))
        })
        .collect();
    Some(Arguments { values, receivers })
}

fn type_surface(ty: &Ty, typed: &TypedFileAst) -> Option<Expr> {
    let canonical_name = |instance: &ast::ty::NamedTypeInstance| {
        typed
            .tag_types
            .iter()
            .filter_map(|(name, ty)| {
                ty.named_instance()
                    .filter(|candidate| candidate.declaration == instance.declaration)
                    .map(|_| name.0)
            })
            .max_by_key(|name| (name.as_str().split('.').count(), *name))
            .unwrap_or(instance.declaration.name)
    };
    Some(match ty {
        Ty::Proposition(_) | Ty::ResultFamily { .. } => {
            Expr::TypeValue(std::sync::Arc::new(ty.clone()))
        }
        Ty::Named { instance, .. } if instance.arguments.is_empty() => {
            Expr::AnonymousTag(canonical_name(instance))
        }
        Ty::Named { instance, .. } => Expr::TagCall(ast::TagCall {
            name: canonical_name(instance),
            qual_path: None,
            args: instance
                .arguments
                .iter()
                .map(|(_, arg)| {
                    let value = match arg {
                        TyArg::Type(ty) => type_surface(ty, typed)?,
                        TyArg::Const(value) => {
                            Expr::Lit(ast::Literal::Int(value.as_const_int()?.into()))
                        }
                    };
                    Some(ast::Typed::infer(value, ast::SpanId::INVALID))
                })
                .collect::<Option<Vec<_>>>()?,
        }),
        Ty::Ptr { inner } | Ty::Address { pointee: inner, .. } => Expr::TakePtr(Box::new(
            ast::Typed::infer(type_surface(inner, typed)?, ast::SpanId::INVALID),
        )),
        Ty::AnonymousInteger { validity } => Expr::IntegerDomain(Box::new(ast::Typed::infer(
            Expr::DomainValue(std::sync::Arc::new(validity.domain().clone())),
            ast::SpanId::INVALID,
        ))),
        Ty::Unit => Expr::ResultValues(Vec::new()),
        Ty::Ref { inner, mutable } => Expr::Ref {
            inner: Box::new(ast::Typed::infer(
                type_surface(inner, typed)?,
                ast::SpanId::INVALID,
            )),
            mutable: *mutable,
            group: None,
        },
        Ty::Array { elem, size } => Expr::FixedArrayAlloc {
            init: Box::new(ast::Typed::infer(
                type_surface(elem, typed)?,
                ast::SpanId::INVALID,
            )),
            size: Box::new(ast::Typed::infer(
                Expr::Lit(ast::Literal::Int(size.as_const_int()?.into())),
                ast::SpanId::INVALID,
            )),
        },
        Ty::Sequence(elements) => Expr::SequenceLiteral(ast::SequenceLiteral::Elements(
            elements
                .iter()
                .map(|element| {
                    Some(ast::SequenceElement {
                        name: element.name,
                        expr: ast::Typed::infer(
                            type_surface(&element.ty, typed)?,
                            ast::SpanId::INVALID,
                        ),
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        )),
        _ => return None,
    })
}

pub(super) struct Arguments {
    pub(super) values: BTreeMap<Intern<String>, Expr>,
    pub(super) receivers: BTreeMap<Intern<String>, ast::ty::TypeId>,
}

pub(super) fn argument_order(
    args: &[ast::Typed<Expr>],
    template: &TypedBind,
) -> Option<Vec<usize>> {
    let names = args
        .iter()
        .map(|arg| match &arg.value {
            Expr::Bind(bind) if bind.params.is_none() => Some(bind.name),
            _ => None,
        })
        .collect::<Vec<_>>();
    crate::staging::argument_order(&names, &template.params)
}
