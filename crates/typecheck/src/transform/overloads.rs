use ast::{BindValue, Expr, FnCall, ModPath, SpanId, Spanned, Ty, TyArg, Typed};
use diagnostic::code::{CallCode, MemberCode, TypeCode};
use diagnostic::{Diagnostic, DiagnosticCode};
use internment::Intern;

use crate::analysis::unify_type_args_with_registry;
use crate::staging::argument_order;
use crate::typed::{DefId, ExprId, TypedCallableSignature, TypedFileAst};

use super::lower_exprs::ExprLowerScope;

pub(super) struct Overloads {
    name: DefId,
    candidates: Vec<(DefId, TypedCallableSignature)>,
}

impl Overloads {
    pub(super) fn for_member(
        member: Intern<String>,
        receiver: &Ty,
        typed: &TypedFileAst,
        scope: &ExprLowerScope<'_>,
    ) -> Option<Self> {
        if let Ty::Ref { inner, .. } = receiver {
            return Self::for_member(member, inner, typed, scope);
        }
        let instance = receiver.named_instance()?;
        let declaration = typed.type_registry.declaration_key(instance)?;
        let mut qualified = vec![declaration.package.name.as_str()];
        qualified.extend(declaration.module.iter().map(|name| name.as_str()));
        qualified.push(declaration.declaration.as_str());
        qualified.push(member.as_str());
        let names = [
            qualified.join("."),
            qualified[1..].join("."),
            format!("{}.{}", receiver.type_name()?, member),
        ];
        for name in names {
            let name = DefId::named(Intern::new(name));
            let methods = Self::find(name, typed, scope).or_else(|| {
                let signature = typed
                    .defs
                    .get(&name)
                    .map(TypedCallableSignature::from)
                    .or_else(|| scope.callable_signatures.get(&name).cloned())?;
                Some(Self {
                    name,
                    candidates: vec![(name, signature)],
                })
            });
            if let Some(mut methods) = methods {
                methods.candidates.retain(|(_, signature)| {
                    signature
                        .params
                        .first()
                        .and_then(|(_, ty)| ty.named_instance_stripping_reference_wrappers())
                        .is_some_and(|candidate| candidate.declaration == instance.declaration)
                });
                if !methods.candidates.is_empty() {
                    return Some(methods);
                }
            }
        }
        None
    }

    pub(super) fn member_call(
        &self,
        source: &Typed<Expr>,
        span: SpanId,
        args: &[Typed<Expr>],
    ) -> Result<FnCall, Diagnostic> {
        if args.is_empty()
            && self
                .candidates
                .iter()
                .all(|(_, signature)| signature.params.len() != 1)
        {
            // type-member-requires-arguments
            return Err(Diagnostic::new(
                MemberCode::ArgumentsRequired,
                format!("method `{}` requires arguments", self.name.0),
            )
            .with_arg("name", self.name.0.as_str().to_string()));
        }
        let mut arguments = vec![source.clone()];
        arguments.extend_from_slice(args);
        Ok(FnCall {
            syntax: ast::CallSyntax::Path,
            definition: None,
            path: Spanned::new(ModPath::new(self.name.0, Vec::new()), span),
            args: Some(arguments),
        })
    }

    pub(super) fn unique_for_arguments(&self, source: &[Typed<Expr>]) -> Option<DefId> {
        if let [(id, _)] = self.candidates.as_slice() {
            return Some(*id);
        }
        let names: Vec<_> = source
            .iter()
            .map(|arg| match &arg.value {
                Expr::Bind(bind) if bind.params.is_none() => Some(bind.name),
                _ => None,
            })
            .collect();
        let mut candidates = self
            .candidates
            .iter()
            .filter(|(_, signature)| argument_order(&names, &signature.params).is_some());
        let (id, _) = candidates.next()?;
        candidates.next().is_none().then_some(*id)
    }

    pub(super) fn find(
        name: DefId,
        typed: &TypedFileAst,
        scope: &ExprLowerScope<'_>,
    ) -> Option<Self> {
        let members = typed
            .overload_groups
            .get(&name.0)
            .or_else(|| scope.overload_groups.get(&name.0))?;
        let candidates = members
            .iter()
            .filter_map(|id| {
                let signature = typed
                    .defs
                    .get(id)
                    .map(TypedCallableSignature::from)
                    .or_else(|| scope.callable_signatures.get(id).cloned())?;
                Some((*id, signature))
            })
            .collect::<Vec<_>>();
        (!candidates.is_empty()).then_some(Self { name, candidates })
    }

    pub(super) fn select(
        &self,
        source: &[Typed<Expr>],
        args: &[ExprId],
        typed: &TypedFileAst,
    ) -> Result<(DefId, TypedCallableSignature, Vec<ExprId>), Diagnostic> {
        let names: Vec<_> = source
            .iter()
            .map(|arg| match &arg.value {
                Expr::Bind(bind) if bind.params.is_none() => Some(bind.name),
                _ => None,
            })
            .collect();
        let mut matches = Vec::new();
        for (id, signature) in &self.candidates {
            let Some(order) = argument_order(&names, &signature.params) else {
                continue;
            };
            let actual: Option<Vec<_>> = order
                .iter()
                .zip(&signature.params)
                .map(|(index, (_, expected))| {
                    argument_type(&source[*index], args[*index], expected, typed)
                        .map(|ty| TyArg::Type(Box::new(ty)))
                })
                .collect();
            let Some(actual) = actual else { continue };
            let expected: Vec<_> = signature
                .params
                .iter()
                .map(|(_, ty)| TyArg::Type(Box::new(ty.clone())))
                .collect();
            let Ok(subst) =
                unify_type_args_with_registry(&expected, &actual, Some(&typed.type_registry))
            else {
                continue;
            };
            if signature
                .params
                .iter()
                .zip(&actual)
                .any(|((_, expected), actual)| {
                    TyArg::Type(Box::new(subst.apply_to_ty(expected))) != *actual
                })
            {
                continue;
            }
            matches.push((
                *id,
                signature.clone(),
                order.into_iter().map(|index| args[index]).collect(),
            ));
        }
        match matches.len() {
            1 => Ok(matches.pop().unwrap()),
            count => {
                let (code, message) = if count == 0 {
                    (
                        // type-no-matching-overload
                        DiagnosticCode::Type(TypeCode::Call(CallCode::OverloadNoMatch)),
                        format!("no overload of `{}` matches these arguments", self.name.0),
                    )
                } else {
                    (
                        // type-ambiguous-overload
                        DiagnosticCode::Type(TypeCode::Call(CallCode::OverloadAmbiguous)),
                        format!(
                            "{count} overloads of `{}` match these arguments",
                            self.name.0
                        ),
                    )
                };
                let signatures = self
                    .candidates
                    .iter()
                    .map(|(_, signature)| {
                        let params = signature
                            .params
                            .iter()
                            .map(|(name, ty)| format!("{name} {}", ty.format_for_hover()))
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("{}({params})", self.name.0)
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                Err(Diagnostic::new(code, message)
                    .with_arg("name", self.name.0.as_str())
                    .with_help(format!("available signatures: {signatures}")))
            }
        }
    }
}

fn argument_type(
    source: &Typed<Expr>,
    arg: ExprId,
    expected: &Ty,
    typed: &TypedFileAst,
) -> Option<Ty> {
    let source = match &source.value {
        Expr::Bind(bind) if let BindValue::Expr(inner) = &bind.value => inner,
        _ => source,
    };
    let actual = &typed.exprs.ty[arg.as_usize()];
    let value_type = match expected {
        Ty::Ref { inner, .. } => inner.as_ref(),
        ty => ty,
    };
    let contextual = matches!(actual, Ty::UnresolvedLiteral(_))
        || matches!(source.value, Expr::Lit(_))
        || matches!(&source.value, Expr::Negate(inner) if matches!(inner.value, Expr::Lit(_)));
    if contextual
        && let Some(ast::ConstValue::Int(value)) =
            typed.exprs.const_value_of(arg).and_then(Option::as_ref)
    {
        return crate::integer_literal::validate_with_registry(
            *value,
            value_type,
            &typed.type_registry,
        )
        .is_ok()
        .then(|| expected.clone());
    }
    match expected {
        Ty::Ref {
            inner: expected_inner,
            mutable,
        } => {
            let inner = match actual {
                Ty::Ref {
                    inner,
                    mutable: actual_mutable,
                } if actual != expected_inner.as_ref() => {
                    if *mutable && !actual_mutable {
                        return None;
                    }
                    inner.clone()
                }
                _ => Box::new(actual.clone()),
            };
            Some(Ty::Ref {
                inner,
                mutable: *mutable,
            })
        }
        _ => Some(actual.clone()),
    }
}
