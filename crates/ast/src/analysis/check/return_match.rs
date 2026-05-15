use std::collections::HashMap;

use diagnostic::DiagnosticLike;
use diagnostic::type_::TypeSymptom;
use internment::Intern;

use crate::analysis::infer::{LocalTypes, TyInfer, TyInferEnv};
use crate::analysis::resolve::{
    is_type_surface, resolve_type_expr_with_subst, typevars_from_receiver,
};
use crate::ty::Ty;
use crate::{Bind, BindValue};

use super::unification::ty_unifies_with;
use super::utils::literal_matches_const_union;

pub(crate) fn check_body_matches_return(
    tag_types: &HashMap<Intern<String>, Ty>,
    fn_return_types: &HashMap<Intern<String>, Ty>,
    bind: &Bind,
    symptoms: &mut Vec<diagnostic::Diagnostic>,
    locals: &dyn LocalTypes,
) {
    let Some(return_tag) = &bind.return_tag else {
        return;
    };
    if !is_type_surface(&return_tag.value) {
        return;
    }

    let subst = bind
        .receiver_type_surface()
        .map(|sp| typevars_from_receiver(&sp.value))
        .unwrap_or_default();
    let expected = resolve_type_expr_with_subst(&return_tag.value, tag_types, &subst);

    let body_ty_and_span: Option<(Ty, crate::SpanId)> = match bind.value() {
        BindValue::Expr(expr) => {
            let env = TyInferEnv {
                tag_types,
                fn_return_types,
                locals,
            };
            Some((expr.infer_ty(&env), expr.span_id))
        }
        BindValue::Body { ret, .. } => ret.value.as_ref().map(|e| {
            let env = TyInferEnv {
                tag_types,
                fn_return_types,
                locals,
            };
            (e.infer_ty(&env), e.span_id)
        }),
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
                    BindValue::Expr(expr) => Some(&expr.value),
                    BindValue::Body { ret, .. } => ret.value.as_ref().map(|spanned| &spanned.value),
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
