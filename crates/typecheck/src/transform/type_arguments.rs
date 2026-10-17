use crate::tag_declarations::TagDeclarations;
use diagnostic::code::GenericCode;
use std::collections::{HashMap, HashSet};

use ast::{Bind, BindValue, Expr, ParameterKind, Parameters, SpanId};
use diagnostic::Diagnostic;
use internment::Intern;

use crate::analysis::expr_is_type_surface;

pub(super) struct TypeArgumentContext<'a> {
    parameters: &'a HashMap<Intern<String>, Parameters>,
    declarations: TagDeclarations<'a>,
    empty_parameters: Parameters,
}

impl<'a> TypeArgumentContext<'a> {
    pub(super) fn new(
        parameters: &'a HashMap<Intern<String>, Parameters>,
        declarations: impl Into<TagDeclarations<'a>>,
    ) -> Self {
        Self {
            parameters,
            declarations: declarations.into(),
            empty_parameters: Parameters::new(),
        }
    }

    fn parameters_for(&self, name: &Intern<String>) -> Option<&Parameters> {
        if let Some(declaration) = self.declarations.get(name) {
            Some(
                declaration
                    .params
                    .as_ref()
                    .unwrap_or(&self.empty_parameters),
            )
        } else {
            self.parameters.get(name)
        }
    }

    pub(super) fn bind_diagnostics(&self, bind: &Bind) -> Vec<(SpanId, Diagnostic)> {
        let mut diagnostics = Vec::new();
        if let Some((name, args)) = &bind.type_annotation {
            let surface = Expr::TagCall(ast::TagCall {
                name: *name,
                qual_path: bind.type_annotation_qual.clone(),
                args: args.clone(),
            });
            if let Some(diagnostic) = self.diagnostic_for(&surface) {
                diagnostics.push((bind.name_span, diagnostic));
            }
        }
        for surface in bind
            .params
            .iter()
            .flat_map(|params| params.values())
            .filter_map(|parameter| match &parameter.kind {
                ParameterKind::Tagged(surface)
                | ParameterKind::ValueParam { ty: surface }
                | ParameterKind::Inferred { ty: surface } => Some(surface.as_ref()),
                _ => None,
            })
            .chain(bind.return_tag.as_deref())
            .chain(bind.receiver_type_surface())
            .chain(bind.named_results.iter().map(|result| result.ty.as_ref()))
        {
            if let Some(diagnostic) = self.diagnostic_for(&surface.value) {
                diagnostics.push((surface.span_id, diagnostic));
            }
        }
        diagnostics
    }

    pub(super) fn declaration_diagnostics(
        &self,
        declaration: &ast::Declare,
    ) -> Vec<(SpanId, Diagnostic)> {
        let mut surfaces: Vec<_> = declaration
            .params
            .iter()
            .flat_map(|params| params.values())
            .filter_map(|parameter| parameter_type_surface(&parameter.kind))
            .collect();
        let mut diagnostics = Vec::new();
        match &declaration.value {
            ast::DeclareValue::Alias(surface) => surfaces.push((surface.span_id, &surface.value)),
            ast::DeclareValue::Has(members) => {
                surfaces.extend(members.iter().filter_map(|member| {
                    match member {
                        ast::HasMember::Property(property) => property
                            .ty
                            .as_ref()
                            .map(|surface| (surface.span_id, &surface.value)),
                        ast::HasMember::Function(_) => None,
                    }
                }));
            }
            ast::DeclareValue::Union { variants } => {
                for variant in variants {
                    if let ast::Pattern::Generic { name, params, .. } = &variant.shape().value {
                        if self.parameters_for(name).is_some() {
                            let args = params
                                .iter()
                                .map(|(name, kind)| match kind {
                                    ParameterKind::Default(value)
                                        if !name.as_str().starts_with("__expr_") =>
                                    {
                                        ast::Typed::infer(
                                            Expr::Bind(Box::new(Bind::new(
                                                *name,
                                                value.span_id,
                                                BindValue::Expr(value.clone()),
                                            ))),
                                            value.span_id,
                                        )
                                    }
                                    ParameterKind::Default(value) => *value.clone(),
                                    ParameterKind::Tagged(surface)
                                    | ParameterKind::ValueParam { ty: surface }
                                    | ParameterKind::Inferred { ty: surface } => {
                                        ast::Typed::infer(surface.value.clone(), surface.span_id)
                                    }
                                    ParameterKind::Proposition(_) => ast::Typed::infer(
                                        Expr::FnCall(ast::FnCall {
                                            syntax: ast::CallSyntax::Path,
                                            definition: None,
                                            path: ast::Spanned::new(
                                                ast::ModPath::new(*name, Vec::new()),
                                                variant.shape().span_id,
                                            ),
                                            args: None,
                                        }),
                                        variant.shape().span_id,
                                    ),
                                    ParameterKind::Generic => ast::Typed::infer(
                                        Expr::AnonymousTag(*name),
                                        variant.shape().span_id,
                                    ),
                                })
                                .collect();
                            let application = Expr::TagCall(ast::TagCall {
                                name: *name,
                                qual_path: None,
                                args,
                            });
                            if let Some(diagnostic) = self.diagnostic_for(&application) {
                                diagnostics.push((variant.shape().span_id, diagnostic));
                            }
                        } else {
                            surfaces.extend(
                                params
                                    .iter()
                                    .filter_map(|(_, kind)| parameter_type_surface(kind)),
                            );
                        }
                    }
                    if let Some(surface) = variant.result_ty() {
                        surfaces.push((surface.span_id, &surface.value));
                    }
                }
            }
            _ => {}
        }
        diagnostics.extend(surfaces.into_iter().filter_map(|(span, expression)| {
            self.diagnostic_for(expression)
                .map(|diagnostic| (span, diagnostic))
        }));
        diagnostics
    }

    pub(super) fn diagnostic_for(&self, expression: &Expr) -> Option<Diagnostic> {
        match expression {
            Expr::TagCall(call) => {
                let parameters = self.parameters_for(&call.name)?;
                if call.args.len() > parameters.len() {
                    // type-excess-generic-arguments
                    return Some(Diagnostic::new(
                        GenericCode::ExcessArguments,
                        format!(
                            "generic type `{}` accepts at most {} arguments, found {}",
                            call.name,
                            parameters.len(),
                            call.args.len()
                        ),
                    ));
                }
                let mut supplied = HashSet::new();
                let mut next_positional = 0;
                for argument in &call.args {
                    let (name, value) = match &argument.value {
                        Expr::Bind(bind) if bind.params.is_none() => match &bind.value {
                            BindValue::Expr(value) => (Some(bind.name), &value.value),
                            _ => (None, &argument.value),
                        },
                        _ => (None, &argument.value),
                    };
                    let (name, parameter) = if let Some(name) = name {
                        let Some(parameter) = parameters.get(&name) else {
                            // type-unknown-generic-argument
                            return Some(Diagnostic::new(
                                GenericCode::UnknownArgument,
                                format!("generic type `{}` has no parameter `{}`", call.name, name),
                            ));
                        };
                        (name, parameter)
                    } else {
                        let (name, parameter) = parameters.get_index(next_positional).expect(
                            "positional argument index is below the validated argument count",
                        );
                        next_positional += 1;
                        (*name, parameter)
                    };
                    if !supplied.insert(name) {
                        // type-duplicate-generic-argument
                        return Some(Diagnostic::new(
                            GenericCode::DuplicateArgument,
                            format!("generic parameter `{name}` is supplied more than once"),
                        ));
                    }
                    let is_type = match &parameter.kind {
                        ParameterKind::Generic => true,
                        ParameterKind::Default(default) => expr_is_type_surface(&default.value),
                        _ => false,
                    };
                    if is_type && let Some(diagnostic) = self.diagnostic_for(value) {
                        return Some(diagnostic);
                    }
                }
                None
            }
            Expr::TakePtr(inner) | Expr::Ref { inner, .. } => self.diagnostic_for(&inner.value),
            Expr::Binary(binary) if binary.op == ast::BinOp::LogicalOr => self
                .diagnostic_for(&binary.lhs.value)
                .or_else(|| self.diagnostic_for(&binary.rhs.value)),
            Expr::FixedArrayAlloc { init, .. } => self.diagnostic_for(&init.value),
            Expr::SequenceLiteral(ast::SequenceLiteral::Elements(elements)) => elements
                .iter()
                .find_map(|element| self.diagnostic_for(&element.value)),
            Expr::SequenceLiteral(ast::SequenceLiteral::Repeat { seed, .. }) => {
                self.diagnostic_for(&seed.value)
            }
            _ => None,
        }
    }
}

fn parameter_type_surface(kind: &ParameterKind) -> Option<(SpanId, &Expr)> {
    match kind {
        ParameterKind::Tagged(surface)
        | ParameterKind::ValueParam { ty: surface }
        | ParameterKind::Inferred { ty: surface } => Some((surface.span_id, &surface.value)),
        ParameterKind::Default(value) if expr_is_type_surface(&value.value) => {
            Some((value.span_id, &value.value))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_diagnostics_preserve_local_overrides_and_package_fallback() {
        let package = parser::parse_from_str(concat!(
            "Pair(package_only) has value package_only\n",
            "Remote(remote_only) has value remote_only\n",
            "Empty(stale) has value stale\n",
        ));
        let parameters = package
            .tags
            .iter()
            .filter_map(|(name, declaration)| {
                declaration
                    .params
                    .clone()
                    .map(|parameters| (*name, parameters))
            })
            .collect();
        for local_parameter in ["local_only", "renamed"] {
            let local = parser::parse_from_str(&format!(
                "Pair({local_parameter}) has value {local_parameter}\nEmpty has value Byte\n",
            ));
            let context = TypeArgumentContext::new(
                &parameters,
                TagDeclarations::overlay(&local.tags, &package.tags),
            );
            for (surface, expected) in [
                (format!("Pair({local_parameter}: Byte)"), None),
                (
                    "Pair(package_only: Byte)".to_owned(),
                    Some("type-unknown-generic-argument"),
                ),
                ("Remote(remote_only: Byte)".to_owned(), None),
                (
                    "Remote(other: Byte)".to_owned(),
                    Some("type-unknown-generic-argument"),
                ),
                (
                    "Empty(Byte)".to_owned(),
                    Some("type-excess-generic-arguments"),
                ),
                ("Missing(anything: Byte)".to_owned(), None),
            ] {
                let parsed = parser::parse_from_str(&format!("Alias is {surface}\n"));
                let ast::DeclareValue::Alias(expression) =
                    &parsed.tags[&Intern::from_ref("Alias")].value
                else {
                    panic!("expected alias type surface")
                };
                assert_eq!(
                    context
                        .diagnostic_for(&expression.value)
                        .map(|diagnostic| diagnostic.code.slug())
                        .as_deref(),
                    expected,
                    "{local_parameter}: {surface}",
                );
            }
        }
    }

    #[test]
    fn sum_type_arguments_are_checked_on_both_sides() {
        let parsed = parser::parse_from_str(
            "Pair(first, second) has left first, right second\nUnknown is Pair(unknown: Byte)\nDuplicate is Pair(first: Byte, first: Byte)\n",
        );
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let name = Intern::from_ref("Pair");
        let parameters = HashMap::from([(name, parsed.tags[&name].params.clone().unwrap())]);
        for (alias, code) in [
            ("Unknown", "type-unknown-generic-argument"),
            ("Duplicate", "type-duplicate-generic-argument"),
        ] {
            let ast::DeclareValue::Alias(surface) = &parsed.tags[&Intern::from_ref(alias)].value
            else {
                panic!("expected alias type surface")
            };
            for invalid_on_left in [true, false] {
                let invalid = ast::Typed::infer(surface.value.clone(), surface.span_id);
                let valid = ast::Typed::infer(
                    Expr::AnonymousTag(Intern::from_ref("Byte")),
                    SpanId::INVALID,
                );
                let (left, right) = if invalid_on_left {
                    (invalid, valid)
                } else {
                    (valid, invalid)
                };
                let sum = Expr::Binary(ast::Binary::new(left, ast::BinOp::LogicalOr, right));
                assert_eq!(
                    TypeArgumentContext::new(&parameters, &parsed.tags)
                        .diagnostic_for(&sum)
                        .expect("invalid argument diagnostic")
                        .code
                        .slug(),
                    code
                );
            }
        }
    }
}
