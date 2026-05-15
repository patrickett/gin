use std::collections::HashMap;

use diagnostic::DiagnosticLike;
use diagnostic::type_::TypeSymptom;
use internment::Intern;

use crate::analysis::infer::{TyInfer, TyInferEnv};
use crate::ty::Ty;
use crate::{Expr, Spanned};

use super::unification::ty_unifies_with;

pub(crate) fn check_call_args(
    params: &[(Intern<String>, Ty)],
    typevars: &HashMap<Intern<String>, Ty>,
    call: &crate::FnCall,
    args: &[Spanned<Expr>],
    symptoms: &mut Vec<diagnostic::Diagnostic>,
    env: &TyInferEnv,
) {
    let mut param_iter = params.iter();
    let mut bindings: HashMap<Intern<String>, Ty> = typevars.clone();
    for arg in args {
        let Some((_pname, param_ty)) = param_iter.next() else {
            break;
        };
        let arg_ty = arg.infer_ty(env);
        if !ty_unifies_with(&arg_ty, param_ty, &mut bindings) {
            symptoms.push(TypeSymptom::Mismatch.into_diagnostic(arg.span_id));
        }
    }
    let _ = call;
}
