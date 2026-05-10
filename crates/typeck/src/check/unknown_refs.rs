use std::ops::ControlFlow;

use ast::visit::{Visitor, walk_expr};
use ast::{Expr, FnCall, HasSpanId, IfCondition, Literal, WhenArm, type_surface_mangle_name};
use diagnostic::type_::TypeSymptom;
use diagnostic::DiagnosticLike;
use internment::Intern;

use crate::flow::ConstValue;
use crate::infer::LayeredLocals;
use crate::resolve::{is_type_surface, mangled_fn_call_name};
use crate::ty::Ty;
use crate::{LocalTypes, TyEnv, TyInfer};

use super::unification::tys_equivalent;
use super::utils::{
    ImportSet, check_type_pattern_default_exprs, fmt_call_without_parens, is_field_of_type,
    suggest_typo_for_identifier, type_name_for_display,
};

pub(crate) struct UnknownRefChecker<'a, 'b, 'c> {
    pub ty_env: &'a TyEnv,
    pub symptoms: &'b mut Vec<diagnostic::Diagnostic>,
    pub locals: &'c dyn LocalTypes,
    pub imports: &'a ImportSet,
}

impl Visitor for UnknownRefChecker<'_, '_, '_> {
    fn visit_fn_call(&mut self, call: &FnCall) -> ControlFlow<()> {
        if !call.path.segments.is_empty()
            && !self.imports.module_prefixes.contains(&call.path.root)
            && !self.imports.alias_spans.contains(&call.path.span_id())
        {
            let mangled = mangled_fn_call_name(call);
            let is_method = self.ty_env.fn_return_ty(&mangled).is_some();
            let is_field_access = call.args.is_none()
                && self
                    .locals
                    .get_type(&call.path.root)
                    .is_some_and(|ty| is_field_of_type(&ty, call.path.segments.last().unwrap()));

            if !is_method && !is_field_access {
                let name = fmt_call_without_parens(call);
                self.symptoms.push(
                    TypeSymptom::UnknownBinding {
                        name,
                        did_you_mean: None,
                    }
                    .into_diagnostic(call.path.span_id()),
                );
                return ControlFlow::Continue(());
            }
        }
        let name = call.path.root;
        let mangled = mangled_fn_call_name(call);
        if let Some(args) = &call.args {
            if self.ty_env.fn_return_ty(&mangled).is_none() {
                let mangled_str = mangled.to_string();
                let suggestion = if !mangled_str.contains('.') {
                    suggest_typo_for_identifier(
                        self.ty_env,
                        &mangled_str,
                        &self.imports.all,
                    )
                } else {
                    None
                };
                self.symptoms.push(
                    TypeSymptom::UnknownBinding {
                        name: mangled_str,
                        did_you_mean: suggestion,
                    }
                    .into_diagnostic(call.path.span_id()),
                );
            }
            for arg in args {
                let _ = self.visit_expr(arg);
            }
            self.ty_env
                .check_call_args(&mangled, call, args, self.symptoms, self.locals);
        } else if call.path.segments.is_empty()
            && self.locals.get_type(&name).is_none()
            && self.ty_env.fn_return_ty(&mangled).is_none()
            && !self.imports.all.contains(&name)
        {
            let suggestion = suggest_typo_for_identifier(
                self.ty_env,
                name.as_str(),
                &self.imports.all,
            );
            self.symptoms.push(
                TypeSymptom::UnknownBinding {
                    name: mangled.to_string(),
                    did_you_mean: suggestion,
                }
                .into_diagnostic(call.path.span_id()),
            );
        } else if call.path.segments.is_empty()
            && call.args.is_none()
            && self.locals.get_type(&name).is_none()
            && self.ty_env.fn_return_ty(&mangled).is_none()
            && self.imports.all.contains(&name)
            && !self.imports.bundle_members.contains(&name)
        {
            self.symptoms.push(
                TypeSymptom::NotExpr {
                    name: name.to_string(),
                }
                .into_diagnostic(call.path.span_id()),
            );
        }
        ControlFlow::Continue(())
    }

    fn visit_when_expr(&mut self, when: &ast::WhenExpr) -> ControlFlow<()> {
        let subject_ty = when
            .subject
            .as_ref()
            .map(|s| s.infer_ty(&self.ty_env.infer_env(self.locals)));
        if let Some(subject) = &when.subject {
            let _ = self.visit_expr(subject);
        }

        let mut has_else = false;

        for arm in &when.arms {
            match arm {
                WhenArm::Cond { condition, body } => {
                    let _ = self.visit_expr(condition);

                    let cond_ty = condition.infer_ty(&self.ty_env.infer_env(self.locals));
                    let bool_ty = self.ty_env.lookup_tag(Intern::<String>::from_ref("Bool"));
                    let is_bool = bool_ty.is_some_and(|bt| tys_equivalent(&cond_ty, bt));
                    if !is_bool {
                        let got = type_name_for_display(&cond_ty);
                        self.symptoms.push(
                            TypeSymptom::ConditionNotBool { got }.into_diagnostic(condition.1),
                        );
                    }

                    let _ = self.visit_expr(body);
                }
                WhenArm::Is { pattern, body } => {
                    match &pattern.0 {
                        Expr::Lit(Literal::String(s)) => {
                            let variant_name = Intern::<String>::from_ref(s.as_str());
                            let valid = match &subject_ty {
                                Some(Ty::ConstUnion {
                                    name: union_name,
                                    values,
                                    ..
                                }) => {
                                    let in_set = values
                                        .iter()
                                        .any(|v| matches!(v, ConstValue::String(vs) if vs == s));
                                    if !in_set {
                                        self.symptoms.push(
                                            TypeSymptom::NotAVariant {
                                                name: s.clone(),
                                                union_name: union_name.to_string(),
                                            }
                                            .into_diagnostic(pattern.1),
                                        );
                                    }
                                    in_set
                                }
                                _ => {
                                    self.ty_env.lookup_variant(variant_name).is_some()
                                }
                            };
                            if !valid {
                                self.symptoms.push(
                                    TypeSymptom::UnknownTag { name: s.clone() }
                                        .into_diagnostic(pattern.1),
                                );
                            }
                        }
                        e if is_type_surface(e) => {
                            let surface_name = type_surface_mangle_name(e);
                            let variant_name = Intern::<String>::from_ref(surface_name);
                            match &subject_ty {
                                Some(Ty::Union {
                                    name: union_name,
                                    variants,
                                }) => {
                                    if !variants.iter().any(|(vname, _)| vname == &variant_name) {
                                        self.symptoms.push(
                                            TypeSymptom::NotAVariant {
                                                name: surface_name.to_string(),
                                                union_name: union_name.to_string(),
                                            }
                                            .into_diagnostic(pattern.1),
                                        );
                                    }
                                }
                                _ => {
                                    if self.ty_env.lookup_variant(variant_name).is_none() {
                                        self.symptoms.push(
                                            TypeSymptom::UnknownTag {
                                                name: surface_name.to_string(),
                                            }
                                            .into_diagnostic(pattern.1),
                                        );
                                    }
                                }
                            }
                            check_type_pattern_default_exprs(e, &mut |expr| {
                                let _ = self.visit_expr(expr);
                            });
                        }
                        _ => {
                            self.symptoms.push(
                                TypeSymptom::UnknownTag {
                                    name: "invalid is-pattern".to_string(),
                                }
                                .into_diagnostic(pattern.1),
                            );
                        }
                    }
                    let _ = self.visit_expr(body);
                }
                WhenArm::Else(body) => {
                    has_else = true;
                    let _ = self.visit_expr(body);
                }
            }
        }

        if !has_else && when.subject.is_none() {
            let span = when
                .arms
                .first()
                .map(|a| match a {
                    WhenArm::Cond { condition, .. } => condition.1,
                    WhenArm::Is { pattern, .. } => pattern.1,
                    WhenArm::Else(body) => body.1,
                })
                .unwrap_or(ast::SpanId::INVALID);
            self.symptoms
                .push(TypeSymptom::MissingElseArm.into_diagnostic(span));
        }

        ControlFlow::Continue(())
    }

    fn visit_if_expr(&mut self, if_expr: &ast::IfExpr) -> ControlFlow<()> {
        match &if_expr.condition {
            IfCondition::Bool(cond) => {
                let _ = self.visit_expr(cond);
                for e in &if_expr.body {
                    let _ = self.visit_expr(e);
                }
            }
            IfCondition::Pattern { subject, pattern } => {
                let _ = self.visit_expr(subject);
                let mut if_locals = LayeredLocals::new(self.locals);
                if is_type_surface(&pattern.0) {
                    if let Expr::TypeGeneric { params, .. } = &pattern.0 {
                        for (k, _) in params.iter() {
                            if k.as_str() != "_" {
                                if_locals.insert(*k, Ty::Opaque(*k));
                            }
                        }
                    }
                    check_type_pattern_default_exprs(&pattern.0, &mut |e| {
                        let _ = self.visit_expr(e);
                    });
                } else {
                    self.symptoms.push(
                        TypeSymptom::UnknownTag {
                            name: "invalid is-pattern".to_string(),
                        }
                        .into_diagnostic(pattern.1),
                    );
                }
                for e in &if_expr.body {
                    let mut inner = UnknownRefChecker {
                        ty_env: self.ty_env,
                        symptoms: &mut *self.symptoms,
                        locals: &if_locals,
                        imports: self.imports,
                    };
                    let _ = inner.visit_expr(e);
                }
            }
        }
        ControlFlow::Continue(())
    }

    fn visit_tag_call(&mut self, tc: &ast::TagCall) -> ControlFlow<()> {
        if let Some(path) = &tc.qual_path {
            if self.ty_env.lookup_tag(path.root).is_none() {
                self.symptoms.push(
                    TypeSymptom::UnknownTag {
                        name: path.root.to_string(),
                    }
                    .into_diagnostic(path.span_id()),
                );
            }
        } else if self.ty_env.lookup_variant(tc.name).is_none() {
            self.symptoms.push(
                TypeSymptom::UnknownTag {
                    name: tc.name.to_string(),
                }
                .into_diagnostic(tc.span_id()),
            );
        }
        for arg in &tc.args {
            let _ = self.visit_expr(arg);
        }
        ControlFlow::Continue(())
    }

    fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        match expr {
            Expr::AnonymousTag(name, span) => {
                if self.ty_env.lookup_variant(*name).is_none() {
                    self.symptoms.push(
                        TypeSymptom::UnknownTag {
                            name: name.to_string(),
                        }
                        .into_diagnostic(*span),
                    );
                }
                ControlFlow::Continue(())
            }
            Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. } => {
                self.ty_env.check_type_expr(expr, self.symptoms);
                ControlFlow::Continue(())
            }
            Expr::Lit(_)
            | Expr::SelfRef(_)
            | Expr::Range(_)
            | Expr::FormatString(_)
            | Expr::Asm(_) => ControlFlow::Continue(()),
            _ => walk_expr(self, expr),
        }
    }
}
