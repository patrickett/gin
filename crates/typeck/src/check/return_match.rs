use std::collections::HashMap;

use ast::{Bind, BindValue};
use diagnostic::type_::TypeSymptom;
use diagnostic::DiagnosticLike;
use internment::Intern;

use crate::resolve::{is_type_surface, resolve_type_expr_with_subst, typevars_from_receiver};
use crate::ty::Ty;
use crate::{LocalTypes, TyEnv, TyInfer};

use super::unification::ty_unifies_with;
use super::utils::literal_matches_const_union;

impl TyEnv {
    pub(crate) fn check_body_matches_return(
        &self,
        bind: &Bind,
        symptoms: &mut Vec<diagnostic::Diagnostic>,
        locals: &dyn LocalTypes,
    ) {
        let Some(return_tag) = &bind.return_tag else {
            return;
        };
        if !is_type_surface(&return_tag.0) {
            return;
        }

        let subst = bind
            .receiver_type_surface()
            .map(|sp| typevars_from_receiver(&sp.0))
            .unwrap_or_default();
        let expected =
            resolve_type_expr_with_subst(&return_tag.0, &self.tag_types, &subst);

        let body_ty_and_span: Option<(Ty, ast::SpanId)> = match bind.value() {
            BindValue::Expr(expr) => Some((expr.infer_ty(&self.infer_env(locals)), expr.1)),
            BindValue::Body { ret, .. } => ret
                .0
                .as_ref()
                .map(|e| (e.infer_ty(&self.infer_env(locals)), e.1)),
            BindValue::Extern => None,
        };

        let Some((body_ty, span)) = body_ty_and_span else {
            return;
        };

        let mut bindings: HashMap<Intern<String>, Ty> = HashMap::new();
        if !ty_unifies_with(&body_ty, &expected, &mut bindings) {
            let is_valid_const_union_lit = match &expected {
                Ty::ConstUnion { values, .. } => {
                    let value_expr = match bind.value() {
                        BindValue::Expr(expr) => Some(&expr.0),
                        BindValue::Body { ret, .. } => ret.0.as_ref().map(|spanned| &spanned.0),
                        _ => None,
                    };
                    value_expr.is_some_and(|e| literal_matches_const_union(e, values))
                }
                _ => false,
            };
            if !is_valid_const_union_lit {
                symptoms.push(TypeSymptom::Mismatch.into_diagnostic(span));
            }
        }
    }
}
