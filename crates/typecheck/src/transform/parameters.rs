use ast::folder::Visitor;
use ast::{Bind, ParamConvention, ParameterKind, Ty};
use diagnostic::code::TraitCode;
use internment::Intern;
use std::ops::ControlFlow;

pub(super) fn inferred_value_type_name(name: Intern<String>) -> Intern<String> {
    Intern::new(format!("parameter$type${name}"))
}

pub(super) fn is_inferred_value(bind: &Bind, name: Intern<String>) -> bool {
    name.as_str() != "self"
        && !bind.param_refinements.contains_key(&name)
        && bind
            .params
            .as_ref()
            .and_then(|params| params.get(&name))
            .is_some_and(|parameter| {
                matches!(parameter.kind, ParameterKind::Generic)
                    && (!parameter.constraints.is_empty()
                        || bind
                            .param_conventions
                            .get(&name)
                            .is_some_and(|convention| *convention != ParamConvention::Terminal)
                        || !used_as_type(bind, name))
            })
}

fn used_as_type(bind: &Bind, name: Intern<String>) -> bool {
    struct Uses(Intern<String>);
    impl Uses {
        fn surface(&self, expr: &ast::Expr) -> ControlFlow<()> {
            struct Names(Intern<String>);
            impl Visitor for Names {
                fn visit_expr(&mut self, expr: &ast::Expr) -> ControlFlow<()> {
                    if matches!(expr, ast::Expr::AnonymousTag(name) if *name == self.0)
                        || matches!(expr, ast::Expr::FnCall(call) if call.path.root == self.0)
                    {
                        return ControlFlow::Break(());
                    }
                    ast::folder::walk_expr(self, expr)
                }
            }
            Names(self.0).visit_expr(expr)
        }

        fn reference(&self, ty: &ast::TypeReference) -> ControlFlow<()> {
            match ty {
                ast::TypeReference::Nominal(name) if *name == self.0 => ControlFlow::Break(()),
                ast::TypeReference::Application { arguments, .. } => {
                    for arg in arguments {
                        self.reference(arg)?;
                    }
                    ControlFlow::Continue(())
                }
                ast::TypeReference::Pointer(inner) => self.reference(inner),
                _ => ControlFlow::Continue(()),
            }
        }
    }
    impl Visitor for Uses {
        fn visit_bind(&mut self, bind: &Bind) -> ControlFlow<()> {
            if bind.return_type_name == Some(self.0) {
                return ControlFlow::Break(());
            }
            for surface in bind.return_tag.iter().chain(bind.receiver_type.iter()) {
                self.surface(&surface.value)?;
            }
            for result in &bind.named_results {
                self.surface(&result.ty.value)?;
            }
            for parameter in bind.params.iter().flat_map(|params| params.values()) {
                match &parameter.kind {
                    ParameterKind::Tagged(surface)
                    | ParameterKind::ValueParam { ty: surface }
                    | ParameterKind::Inferred { ty: surface } => self.surface(&surface.value)?,
                    _ => {}
                }
            }
            if let Some((_, args)) = &bind.type_annotation {
                for arg in args {
                    self.surface(&arg.value)?;
                }
            }
            ast::folder::walk_bind(self, bind)
        }

        fn visit_expr(&mut self, expr: &ast::Expr) -> ControlFlow<()> {
            if let ast::Expr::Cast { ty, .. } | ast::Expr::TargetQuery { operand: ty, .. } = expr {
                self.reference(ty)?;
            }
            ast::folder::walk_expr(self, expr)
        }
    }
    Uses(name).visit_bind(bind).is_break()
}

pub(super) fn inferred_value_type(bind: &Bind, name: Intern<String>) -> Ty {
    let ty = Ty::Opaque(inferred_value_type_name(name));
    match bind.param_conventions.get(&name) {
        Some(ParamConvention::Observe) => Ty::Ref {
            inner: Box::new(ty),
            mutable: false,
        },
        Some(ParamConvention::Mutate) => Ty::Ref {
            inner: Box::new(ty),
            mutable: true,
        },
        _ => ty,
    }
}

pub(super) fn constraint_diagnostics(
    bind: &Bind,
    local: &ast::TagMap,
    package: &ast::TagMap,
) -> Vec<(ast::SpanId, diagnostic::Diagnostic)> {
    bind.params
        .iter()
        .flat_map(|params| params.values())
        .flat_map(|parameter| &parameter.constraints)
        .filter_map(|constraint| {
            let declaration = local
                .get(&constraint.trait_name)
                .or_else(|| package.get(&constraint.trait_name));
            let diagnostic = match declaration {
                // type-unknown-trait
                None => diagnostic::Diagnostic::new(
                    TraitCode::Unknown,
                    format!("unknown trait `{}`", constraint.trait_name),
                ),
                Some(declaration) => {
                    let expected = declaration.params.as_ref().map_or(0, |params| params.len());
                    if constraint.args.len() == expected {
                        return None;
                    }
                    // type-trait-argument-count
                    diagnostic::Diagnostic::new(
                        TraitCode::ArgumentCount,
                        format!(
                            "trait `{}` requires {expected} arguments, found {}",
                            constraint.trait_name,
                            constraint.args.len()
                        ),
                    )
                }
            };
            Some((constraint.trait_name_span, diagnostic))
        })
        .collect()
}

pub(super) fn resolve_constraints(bind: &Bind, env: &crate::analysis::TypeEnv<'_>) -> Vec<Vec<Ty>> {
    bind.params
        .iter()
        .flat_map(|params| params.values())
        .map(|parameter| {
            parameter
                .constraints
                .iter()
                .map(|constraint| {
                    env.resolve_expr(&ast::Expr::TagCall(ast::TagCall {
                        name: constraint.trait_name,
                        qual_path: None,
                        args: constraint.args.clone(),
                    }))
                })
                .collect()
        })
        .collect()
}
