use std::collections::HashMap;

use ast::{Expr, Spanned};
use diagnostic::type_::TypeSymptom;
use diagnostic::DiagnosticLike;
use internment::Intern;

use crate::ty::Ty;
use crate::{LocalTypes, TyEnv, TyInfer};

use super::unification::ty_unifies_with;

impl TyEnv {
    pub(crate) fn check_call_args(
        &self,
        mangled: &Intern<String>,
        call: &ast::FnCall,
        args: &[Spanned<Expr>],
        symptoms: &mut Vec<diagnostic::Diagnostic>,
        locals: &dyn LocalTypes,
    ) {
        let Some(info) = self.fn_params.get(mangled) else {
            eprintln!(
                "DEBUG check_call_args: {mangled} NOT in fn_params, keys: {:?}",
                self.fn_params
                    .keys()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
            );
            return;
        };
        let mut params = info.params.iter();
        let mut bindings: HashMap<Intern<String>, Ty> = info.typevars.clone();
        for arg in args {
            let Some((pname, param_ty)) = params.next() else {
                break;
            };
            let arg_ty = arg.infer_ty(&self.infer_env(locals));
            eprintln!(
                "DEBUG check_call_args[{mangled}]: param={pname} param_ty={param_ty:?} arg_ty={arg_ty:?}"
            );
            if !ty_unifies_with(&arg_ty, param_ty, &mut bindings) {
                eprintln!("DEBUG check_call_args[{mangled}]: MISMATCH");
                symptoms.push(TypeSymptom::Mismatch.into_diagnostic(arg.1));
            }
        }
        let _ = call;
    }
}
