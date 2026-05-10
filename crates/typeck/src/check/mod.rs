//! AST validation — unknown-reference checking, return-type matching,
//! call-argument checking, type-surface validation, and unused-binding detection.

mod unknown_refs;
mod return_match;
mod call_args;
mod type_expr;
mod variant_return;
mod unused_bindings;
mod unification;
mod utils;

use std::collections::HashMap;

use ast::visit::Visitor;
use ast::{Bind, BindValue, Expr, FileAst, type_surface_mangle_name};
use internment::Intern;

use crate::resolve::{is_type_surface, typevars_from_receiver};
use crate::ty::Ty;
use crate::{LocalTypes, TyEnv, TyInfer};

use unknown_refs::UnknownRefChecker;
use utils::{collect_import_names, ImportSet};

pub use unification::ty_unifies_with;

impl TyEnv {
    /// Check a single file's AST for unknown references, type mismatches,
    /// and unused bindings. Appends diagnostics to `symptoms`.
    pub fn check_unknowns(&self, ast: &FileAst, symptoms: &mut Vec<diagnostic::Diagnostic>) {
        let imports = collect_import_names(ast);
        for bind in ast.defs.values() {
            if !bind.attributes().matches_current_platform() {
                continue;
            }
            let subst = bind
                .receiver_type_surface()
                .map(|sp| typevars_from_receiver(&sp.0))
                .unwrap_or_default();
            let mut locals: HashMap<Intern<String>, Ty> = HashMap::new();
            if let Some(params) = bind.params() {
                for (name, kind) in params.iter() {
                    locals.insert(
                        *name,
                        self.resolve_parameter_kind_with_subst(*name, kind, &subst),
                    );
                }
            }
            if let Some(sp) = bind.receiver_type_surface()
                && is_type_surface(&sp.0)
            {
                let recv_ty =
                    crate::resolve::resolve_type_expr_with_subst(&sp.0, &self.tag_types, &subst);
                locals.insert(Intern::<String>::from_ref("self"), recv_ty);
            }
            self.check_bind(bind, symptoms, &locals, &imports);
        }
    }

    fn check_bind(
        &self,
        bind: &Bind,
        symptoms: &mut Vec<diagnostic::Diagnostic>,
        locals: &dyn LocalTypes,
        imports: &ImportSet,
    ) {
        if let Some(sp) = &bind.return_tag
            && is_type_surface(&sp.0)
        {
            self.check_type_expr(&sp.0, symptoms);
            if let Some(Ty::Union {
                name: union_name,
                variants,
            }) = self.lookup_tag(Intern::<String>::from_ref(type_surface_mangle_name(&sp.0)))
            {
                let valid_variants: Vec<Intern<String>> =
                    variants.iter().map(|(vname, _)| *vname).collect();
                variant_return::check_return_variants(bind, &valid_variants, *union_name, symptoms);
            }
        }
        self.check_body_matches_return(bind, symptoms, locals);
        match bind.value() {
            BindValue::Expr(expr) => {
                let mut checker = UnknownRefChecker {
                    ty_env: self,
                    symptoms,
                    locals,
                    imports,
                };
                let _ = checker.visit_expr(expr);
            }
            BindValue::Body { exprs, ret } => {
                let mut body_locals = crate::infer::LayeredLocals::new(locals);
                for expr in exprs.iter() {
                    if let Expr::Bind(inner) = &**expr {
                        self.check_bind(inner, symptoms, &body_locals, imports);
                        body_locals.insert(inner.name(), {
                            let env = crate::TyInferEnv {
                                tag_types: &self.tag_types,
                                fn_return_types: &self.fn_return_types,
                                locals: &HashMap::new(),
                            };
                            inner.infer_ty(&env)
                        });
                    } else {
                        let mut checker = UnknownRefChecker {
                            ty_env: self,
                            symptoms,
                            locals: &body_locals,
                            imports,
                        };
                        let _ = checker.visit_expr(expr);
                    }
                }
                if let Some(ret_expr) = &ret.0 {
                    let mut checker = UnknownRefChecker {
                        ty_env: self,
                        symptoms,
                        locals: &body_locals,
                        imports,
                    };
                    let _ = checker.visit_expr(ret_expr);
                }

                unused_bindings::detect_unused_bindings(exprs, &ret.0, symptoms);
            }
            BindValue::Extern => {}
        }
    }
}
