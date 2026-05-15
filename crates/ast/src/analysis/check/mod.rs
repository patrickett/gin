//! AST validation — unknown-reference checking, return-type matching,
//! call-argument checking, type-surface validation, and unused-binding detection.

mod call_args;
mod return_match;
mod type_expr;
mod unification;
mod unknown_refs;
mod unused_bindings;
mod utils;
mod variant_return;

pub mod pipeline;

use std::collections::{HashMap, HashSet};

use internment::Intern;

use crate::analysis::infer::{TyInfer, TyInferEnv};
use crate::analysis::resolve::{is_type_surface, typevars_from_receiver};
use crate::ty::Ty;
use crate::visit::Visitor;
use diagnostic::{DiagnosticLike, UseSymptom};

use crate::{
    Bind, BindValue, DeclareValue, Expr, FileAst, ImportSource, ParameterKind, VariantMap,
    type_surface_mangle_name,
};
use unknown_refs::UnknownRefChecker;
use utils::{ImportSet, collect_import_names};

/// Check a single file's AST for unknown references, type mismatches,
/// and unused bindings. Appends diagnostics to `symptoms`.
pub fn check_unknowns(
    ast: &FileAst,
    tag_types: &HashMap<Intern<String>, Ty>,
    explicit_tag_names: &HashSet<Intern<String>>,
    fn_return_types: &HashMap<Intern<String>, Ty>,
    variant_map: &VariantMap,
    symptoms: &mut Vec<diagnostic::Diagnostic>,
) {
    let imports = collect_import_names(ast);

    // Validate CurrentModule imports: each name must exist as a tag in the package.
    for imp in ast.uses() {
        for mi in &imp.0 {
            if let ImportSource::CurrentModule { member } = &mi.source {
                let name = member.alias.unwrap_or(member.export);
                if !explicit_tag_names.contains(&name) {
                    symptoms.push(
                        UseSymptom::NotExported {
                            symbol: name.to_string(),
                            module: "current module".to_string(),
                        }
                        .into_diagnostic(member.span),
                    );
                }
            }
        }
    }

    let local_names: HashSet<Intern<String>> =
        ast.tags.keys().chain(ast.defs.keys()).copied().collect();
    // Check tag field type annotations for unknown references.
    for tag in ast.tags.values() {
        let DeclareValue::Record(fields) = tag.value() else {
            continue;
        };

        // Collect type variable names from the tag's own params
        // (e.g. `x` in `List[x] has (pointer Ptr[x], length Int)`)
        let mut type_vars: HashSet<Intern<String>> = HashSet::new();
        if let Some(params) = &tag.params {
            for (name, kind) in params {
                if matches!(kind, ParameterKind::Generic) {
                    type_vars.insert(*name);
                }
            }
        }

        for (_field_name, kind) in fields {
            if let ParameterKind::Tagged(sp) = kind
                && let Some(te) = sp.value.as_type_expr()
                && is_type_surface(&te)
            {
                type_expr::check_type_expr(
                    tag_types,
                    &imports,
                    &local_names,
                    &type_vars,
                    &te,
                    symptoms,
                );
            }
        }
    }

    for bind in ast.defs.values() {
        if !bind.attributes().matches_current_platform() {
            continue;
        }
        let subst = bind
            .receiver_type_surface()
            .map(|sp| typevars_from_receiver(&sp.value))
            .unwrap_or_default();
        let mut locals: HashMap<Intern<String>, Ty> = HashMap::new();
        if let Some(params) = bind.params() {
            for (name, kind) in params.iter() {
                locals.insert(
                    *name,
                    crate::resolve_parameter_kind_with_subst(
                        *name,
                        kind,
                        tag_types,
                        fn_return_types,
                        &subst,
                    ),
                );
            }
        }
        if let Some(sp) = bind.receiver_type_surface()
            && is_type_surface(&sp.value)
        {
            let recv_ty = crate::analysis::resolve::resolve_type_expr_with_subst(
                &sp.value, tag_types, &subst,
            );
            locals.insert(Intern::<String>::from_ref("self"), recv_ty);
        }
        let env = TyInferEnv {
            tag_types,
            fn_return_types,
            locals: &locals,
        };
        check_bind(
            ast,
            bind,
            symptoms,
            &imports,
            &local_names,
            variant_map,
            &env,
        );
    }
}

fn check_bind(
    ast: &FileAst,
    bind: &Bind,
    symptoms: &mut Vec<diagnostic::Diagnostic>,
    imports: &ImportSet,
    local_names: &HashSet<Intern<String>>,
    variant_map: &VariantMap,
    env: &TyInferEnv,
) {
    // Collect type variable names (e.g. `x` in `Range[x].new(start x, end x)`)
    // so check_type_expr can skip them — they are not concrete tag names.
    let mut type_vars: HashSet<Intern<String>> = HashSet::new();
    if let Some(sp) = bind.receiver_type_surface()
        && let crate::TypeExpr::Generic { params, .. } = &sp.value
    {
        for (name, kind) in params {
            if matches!(kind, crate::ParameterKind::Generic) {
                type_vars.insert(*name);
            }
        }
    }

    if let Some(sp) = &bind.return_tag
        && is_type_surface(&sp.value)
    {
        type_expr::check_type_expr(
            env.tag_types,
            imports,
            local_names,
            &type_vars,
            &sp.value,
            symptoms,
        );
        if let Some(Ty::Union {
            name: union_name,
            variants,
        }) = env
            .tag_types
            .get(&Intern::<String>::from_ref(type_surface_mangle_name(
                &sp.value,
            )))
        {
            let valid_variants: Vec<Intern<String>> =
                variants.iter().map(|(vname, _)| *vname).collect();
            variant_return::check_return_variants(bind, &valid_variants, *union_name, symptoms);
        }
    }

    // Check parameter type annotations against imports.
    if let Some(params) = bind.params() {
        for (_name, kind) in params.iter() {
            if let crate::ParameterKind::Tagged(sp) = kind
                && let Some(te) = sp.value.as_type_expr()
                && is_type_surface(&te)
            {
                type_expr::check_type_expr(
                    env.tag_types,
                    imports,
                    local_names,
                    &type_vars,
                    &te,
                    symptoms,
                );
            }
        }
    }

    return_match::check_body_matches_return(
        env.tag_types,
        env.fn_return_types,
        bind,
        symptoms,
        env.locals,
    );
    match bind.value() {
        BindValue::Expr(expr) => {
            let mut checker = UnknownRefChecker {
                ast,
                tag_types: env.tag_types,
                fn_return_types: env.fn_return_types,
                variant_map,
                symptoms,
                locals: env.locals,
                imports,
            };
            let _ = checker.visit_expr(expr);
        }
        BindValue::Body { exprs, ret } => {
            let mut body_locals = crate::analysis::infer::LayeredLocals::new(env.locals);
            for expr in exprs.iter() {
                if let Expr::Bind(inner) = &**expr {
                    let inner_env = TyInferEnv {
                        tag_types: env.tag_types,
                        fn_return_types: env.fn_return_types,
                        locals: &body_locals,
                    };
                    check_bind(
                        ast,
                        inner,
                        symptoms,
                        imports,
                        local_names,
                        variant_map,
                        &inner_env,
                    );
                    body_locals.insert(inner.name(), {
                        let infer_env = TyInferEnv {
                            tag_types: env.tag_types,
                            fn_return_types: env.fn_return_types,
                            locals: &HashMap::new(),
                        };
                        inner.infer_ty(&infer_env)
                    });
                } else {
                    let mut checker = UnknownRefChecker {
                        ast,
                        tag_types: env.tag_types,
                        fn_return_types: env.fn_return_types,
                        variant_map,
                        symptoms,
                        locals: &body_locals,
                        imports,
                    };
                    let _ = checker.visit_expr(expr);
                }
            }
            if let Some(ret_expr) = &ret.value {
                let mut checker = UnknownRefChecker {
                    ast,
                    tag_types: env.tag_types,
                    fn_return_types: env.fn_return_types,
                    variant_map,
                    symptoms,
                    locals: &body_locals,
                    imports,
                };
                let _ = checker.visit_expr(ret_expr);
            }

            unused_bindings::detect_unused_bindings(exprs, &ret.value, symptoms);
        }
        BindValue::Extern => {}
    }
}
