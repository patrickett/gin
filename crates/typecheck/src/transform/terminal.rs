use diagnostic::code::TerminalCode;
use std::collections::HashSet;

use ast::ty::{DeclarationKey, TyArg};
use ast::{DeclareValue, Expr, FileAst, HasMember, NormalExpr, ParamConvention, ParameterKind, Ty};
use diagnostic::{Category, Diagnostic};

use super::TransformCtx;
use super::trait_methods::TraitMethods;
use crate::typed::TypedFileAst;

impl TypedFileAst {
    fn validate_redundant_terminal_methods(&mut self, file: &FileAst, ctx: &TransformCtx) {
        let methods = TraitMethods::new(file, &ctx.compile_time_eval_ast);
        for (id, source) in &file.defs {
            if !matches!(&source.value, ast::BindValue::Body { exprs, ret }
                if exprs.is_empty() && ret.values.is_empty())
            {
                continue;
            }
            let Some(bind) = self.defs.get(id) else {
                continue;
            };
            let Some(receiver) = &bind.receiver_type else {
                continue;
            };
            if bind.params.len() != 1
                || bind.params[0].0.as_str() != "self"
                || &bind.params[0].1 != receiver
                || bind.param_conventions.first() != Some(&ParamConvention::Terminal)
                || bind.return_type != Ty::Unit
            {
                continue;
            }
            if !matches!(self.type_registry.resolved_definition_for_type(receiver),
                Ty::Union { variants, .. } if variants.iter().all(|variant|
                    variant.kind == ast::ty::UnionVariantKind::Constructor
                        && variant.fields.is_empty()))
            {
                continue;
            }
            if !methods.contracts(source).is_empty() {
                continue;
            }
            self.warnings.push(
                // type-redundant-terminal-method
                Diagnostic::new(
                    TerminalCode::MethodRedundant,
                    "an empty terminal method is unnecessary for a payload-free sum",
                )
                .with_category(Category::Help)
                .with_help(
                    "payload-free sums need no terminal exit; remove this method if it is unused",
                )
                .at_span_id(bind.name_span, &self.span_table),
            );
        }
    }

    pub(super) fn terminal_exit_declarations(&self) -> HashSet<DeclarationKey> {
        let mut exits = HashSet::new();
        for bind in self.defs.values() {
            let Some(ty) = &bind.receiver_type else {
                continue;
            };
            for (slot, ((name, param_ty), convention)) in
                bind.params.iter().zip(&bind.param_conventions).enumerate()
            {
                if name.as_str() != "self"
                    || param_ty != ty
                    || *convention != ParamConvention::Terminal
                    || bind
                        .param_refinements
                        .get(slot)
                        .is_some_and(Option::is_some)
                    || !self.owns_terminal_authority(ty)
                {
                    continue;
                }
                let Ty::Named { instance, .. } = ty else {
                    continue;
                };
                let Some(declaration) = self.type_registry.declaration_for_type(ty) else {
                    continue;
                };
                let mut parameters = HashSet::new();
                let universal = instance.arguments.len() == declaration.parameters.len()
                    && instance.arguments.iter().all(|(_, argument)| {
                        let parameter = match argument {
                            TyArg::Type(ty) => match ty.as_ref() {
                                Ty::Opaque(name) => name,
                                _ => return false,
                            },
                            TyArg::Const(NormalExpr::Var(name)) => name,
                            _ => return false,
                        };
                        parameters.insert(*parameter)
                    });
                if universal {
                    exits.insert(declaration.key.clone());
                }
            }
        }
        exits
    }

    pub(super) fn validate_terminal_declarations(&mut self, file: &FileAst, ctx: &TransformCtx) {
        let _profile = compiler_profile::span(compiler_profile::Phase::Terminals);
        self.validate_redundant_terminal_methods(file, ctx);
        let mut exits = self.terminal_exit_declarations();
        exits.extend(ctx.cross_file_terminal_exits.iter().cloned());
        self.type_registry.set_terminal_exits(exits.iter().cloned());
        for declaration in file.tags.values() {
            let Some(tag) = self.tags.get(&crate::typed::TagId(declaration.name)) else {
                continue;
            };
            let Some(identity) = self.type_registry.declaration_for_type(&tag.resolved_ty) else {
                continue;
            };
            let fieldless = identity.sequence_literal.is_none()
                && matches!(&identity.definition, Ty::Record { fields, .. } if fields.is_empty());
            let interface = fieldless
                && TraitMethods::new(file, &ctx.compile_time_eval_ast).is_interface(declaration);
            if identity.display_name == declaration.name
                && interface
                && let DeclareValue::Has(members) = &declaration.value
            {
                for member in members {
                    let HasMember::Function(method) = member else {
                        continue;
                    };
                    let terminal = method.params.iter().any(|(name, param)| {
                        name.as_str() == "self"
                            && method.conventions.get(name).copied().unwrap_or_default()
                                == ParamConvention::Terminal
                            && match &param.kind {
                                ParameterKind::Generic => true,
                                ParameterKind::Tagged(surface) =>
                                    matches!(&surface.value, Expr::AnonymousTag(name) if name.as_str() == "Self")
                                        || surface.value == declaration.receiver_surface().value,
                                _ => false,
                            }
                    });
                    let empty = matches!(
                        &method.body,
                        Some(ast::HasMemberBody::Overrideable(ast::BindValue::Body { exprs, ret })
                            | ast::HasMemberBody::Final(ast::BindValue::Body { exprs, ret }))
                            if exprs.is_empty() && ret.values.is_empty()
                    );
                    if terminal && empty {
                        self.declaration_flaws.push((
                            method.name_span,
                            // type-empty-terminal-default
                            Diagnostic::new(
                                TerminalCode::EmptyDefault,
                                "an interface must not provide an empty terminal default",
                            )
                            .with_help("declare a terminal requirement without a body instead")
                            .at_span_id(method.name_span, &self.span_table),
                        ));
                    }
                }
            }
            if identity.display_name != declaration.name
                || exits.contains(&identity.key)
                || fieldless
                || matches!(&identity.definition, Ty::Union { .. } | Ty::Record { .. })
            {
                continue;
            }
            let inherited =
                TraitMethods::new(file, &ctx.compile_time_eval_ast).required(declaration);
            let members = match &declaration.value {
                DeclareValue::Has(members) => members.as_slice(),
                _ => &[],
            };
            let terminal_contract = members
                .iter()
                .filter_map(|member| match member {
                    HasMember::Function(method) => Some(method.as_ref()),
                    _ => None,
                })
                .chain(inherited.iter().map(|contract| &contract.method))
                .any(|method| {
                    let Some((name, param)) = method.params.iter().find(|(name, _)| name.as_str() == "self") else {
                        return false;
                    };
                    if method.body.is_some()
                        || method.param_refinements.contains_key(name)
                        || method.conventions.get(name).copied().unwrap_or_default() != ParamConvention::Terminal
                    {
                        return false;
                    }
                    match &param.kind {
                        ParameterKind::Generic => true,
                        ParameterKind::Tagged(surface) =>
                            matches!(&surface.value, Expr::AnonymousTag(name) if name.as_str() == "Self")
                                || surface.value == declaration.receiver_surface().value,
                        _ => false,
                    }
                });
            if terminal_contract {
                continue;
            }
            self.declaration_flaws.push((
                declaration.name_span,
                // type-terminal-exit-required
                Diagnostic::new(
                    TerminalCode::ExitRequired,
                    format!(
                        "type `{}` must expose at least one terminal exit in its defining module",
                        declaration.name
                    ),
                )
                .with_arg("type_name", declaration.name.as_str())
                .with_help("declare a method with terminal `self` (without `ref`, `mut`, or `own`)")
                .at_span_id(declaration.name_span, &self.span_table),
            ));
        }
    }
}
