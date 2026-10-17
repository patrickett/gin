use std::collections::HashSet;

use ast::{Expr, Ty, TyArg};

use crate::analysis::TypeEnv;
use crate::normal_expr::Normalize;
use crate::typed::{TypedBind, TypedFileAst};

use super::arguments::Arguments;

pub(super) fn validate(
    template: &TypedBind,
    arguments: &Arguments,
    typed: &TypedFileAst,
    declarations: &ast::TagMap,
) -> Result<(), String> {
    if template.param_constraints.iter().all(Vec::is_empty) {
        return Ok(());
    }
    let tags = typed
        .tag_types
        .iter()
        .map(|(name, ty)| (name.0, ty.clone()))
        .collect();
    let tag_params = declarations
        .iter()
        .filter_map(|(name, declaration)| {
            declaration
                .params
                .as_ref()
                .map(|params| (*name, params.clone()))
        })
        .collect();
    let env = TypeEnv::new(&tags)
        .with_registry(&typed.type_registry)
        .with_tag_decls(declarations)
        .with_tag_params(&tag_params);
    let subst = arguments
        .values
        .iter()
        .map(|(name, value)| (*name, env.resolve_expr(value)))
        .collect();
    let subst = crate::subst::DepSubst {
        types: subst,
        ..Default::default()
    };
    let conformance = Conformance {
        typed,
        declarations,
        env: env.with_subst(&subst.types),
    };
    for ((name, ty), constraints) in template
        .params
        .iter()
        .zip(template.param_constraints.iter())
    {
        for constraint in constraints {
            let subject = subst.apply_to_ty(ty);
            let subject = match subject {
                Ty::Ref { inner, .. } => *inner,
                ty => ty,
            };
            let required = subst.apply_to_ty(constraint);
            if !conformance.provides(&subject, &required, &mut HashSet::new()) {
                return Err(format!(
                    "parameter `{name}` requires {} has {}",
                    subject.format_for_hover(),
                    required.format_for_hover()
                ));
            }
        }
    }
    Ok(())
}

struct Conformance<'a> {
    typed: &'a TypedFileAst,
    declarations: &'a ast::TagMap,
    env: TypeEnv<'a>,
}

impl Conformance<'_> {
    fn resolve(&self, provided: &ast::ProvidedTrait) -> Ty {
        if provided.args.is_empty() {
            self.env
                .resolve_expr(&Expr::AnonymousTag(provided.trait_name))
        } else {
            self.env.resolve_expr(&Expr::TagCall(ast::TagCall {
                name: provided.trait_name,
                qual_path: None,
                args: provided.args.clone(),
            }))
        }
    }

    fn provides(&self, subject: &Ty, required: &Ty, seen: &mut HashSet<ast::ty::TypeId>) -> bool {
        if same_trait(subject, required) {
            return true;
        }
        let Some(instance) = subject.named_instance() else {
            return false;
        };
        let Some(declaration) = self.typed.type_registry.declaration_for_instance(instance) else {
            return false;
        };
        let Some(source) = self.declarations.get(&declaration.display_name) else {
            return false;
        };
        if !seen.insert(instance.declaration) {
            return false;
        }
        let subst = crate::subst::DepSubst::from_ty_args(&instance.arguments);
        let conformance = Conformance {
            typed: self.typed,
            declarations: self.declarations,
            env: self.env.with_subst(&subst.types),
        };
        let provided = source.provided_traits.iter().any(|provided| {
            let provided = subst.apply_to_ty(&conformance.resolve(provided));
            conformance.provides(&provided, required, seen)
        });
        seen.remove(&instance.declaration);
        provided
    }
}

fn same_trait(subject: &Ty, required: &Ty) -> bool {
    if subject == required {
        return true;
    }
    match (subject, required) {
        (
            Ty::Named {
                instance: subject, ..
            },
            Ty::Named {
                instance: required, ..
            },
        ) => {
            subject.declaration == required.declaration
                && subject.arguments.len() == required.arguments.len()
                && subject.arguments.iter().zip(&required.arguments).all(
                    |((subject_name, subject), (required_name, required))| {
                        subject_name == required_name
                            && match (subject, required) {
                                (TyArg::Type(subject), TyArg::Type(required)) => {
                                    same_trait(subject, required)
                                }
                                (TyArg::Const(subject), TyArg::Const(required)) => {
                                    subject.normalize() == required.normalize()
                                }
                                _ => false,
                            }
                    },
                )
        }
        _ => subject == required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use internment::Intern;
    use parser::query::SourceParseExt;

    #[test]
    fn trait_equivalence_preserves_identity_and_argument_contracts() {
        let named = |file, parameter: &str, argument| {
            let name = Intern::from_ref("Sized");
            Ty::Named {
                name,
                instance: ast::ty::NamedTypeInstance::new(ast::ty::TypeId { file, name })
                    .with_arguments(vec![(Intern::from_ref(parameter), argument)]),
            }
        };
        let constant = |value: i128| TyArg::Const(value.into());
        let concrete = named(0, "size", constant(4));
        let arithmetic = named(
            0,
            "size",
            TyArg::Const(ast::NormalExpr::Add(
                Box::new(3_i128.into()),
                Box::new(1_i128.into()),
            )),
        );
        assert!(same_trait(&arithmetic, &concrete));
        assert!(same_trait(
            &named(0, "element", TyArg::Type(Box::new(arithmetic))),
            &named(0, "element", TyArg::Type(Box::new(concrete.clone()))),
        ));
        for other in [
            named(1, "size", constant(4)),
            named(0, "other", constant(4)),
            named(0, "size", TyArg::Type(Box::new(Ty::Unit))),
            named(
                0,
                "size",
                TyArg::Const(ast::NormalExpr::Var(Intern::from_ref("n"))),
            ),
        ] {
            assert!(!same_trait(&concrete, &other));
            assert!(!same_trait(&other, &concrete));
        }
        let mut missing = concrete.clone();
        let Ty::Named { instance, .. } = &mut missing else {
            unreachable!()
        };
        instance.arguments.clear();
        assert!(!same_trait(&concrete, &missing));
    }

    #[test]
    fn inherited_traits_preserve_constant_arguments() {
        let mut parsed = "Int is in 0 to 255\nSized(size Int) has\n    size(self ref) Int\nWrapper(length Int) has Sized(length)\nCarrier(count Int) has Wrapper(count)\nOffset(length Int) has Sized(length + 1)\nScaled(length Int) has Offset(2 * length)\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        crate::prepare_parse_ast(&mut parsed.ast, &flask::CompileTarget::Library);
        let typed = crate::transform::stage_declare(
            &parsed.ast,
            crate::FileId(0),
            &crate::transform::TransformCtx::new(),
        );
        let tags = typed
            .tag_types
            .iter()
            .map(|(name, ty)| (name.0, ty.clone()))
            .collect();
        let params = parsed
            .ast
            .tags
            .iter()
            .filter_map(|(name, declaration)| {
                declaration
                    .params
                    .as_ref()
                    .map(|params| (*name, params.clone()))
            })
            .collect();
        let env = TypeEnv::new(&tags)
            .with_registry(&typed.type_registry)
            .with_tag_decls(&parsed.ast.tags)
            .with_tag_params(&params);
        let conformance = Conformance {
            typed: &typed,
            declarations: &parsed.ast.tags,
            env,
        };
        let application = |name: &str, length: i128| {
            conformance.env.resolve_expr(&Expr::TagCall(ast::TagCall {
                name: Intern::from_ref(name),
                qual_path: None,
                args: vec![ast::Typed::infer(
                    Expr::Lit(ast::Literal::Int(length.into())),
                    ast::SpanId::INVALID,
                )],
            }))
        };
        for (name, expected) in [("Wrapper", 3), ("Carrier", 3), ("Offset", 4), ("Scaled", 7)] {
            let subject = application(name, 3);
            assert!(
                conformance.provides(
                    &subject,
                    &application("Sized", expected),
                    &mut HashSet::new()
                ),
                "{name}"
            );
            assert!(
                !conformance.provides(
                    &subject,
                    &application("Sized", expected + 1),
                    &mut HashSet::new()
                ),
                "{name}"
            );
        }
    }
}
