use std::collections::HashMap;
use std::ops::ControlFlow;

use crate::visit::{Visitor, walk_expr};
use crate::{Expr, FnCall, HasSpanId, IfCondition, TypeExpr, WhenArm, type_surface_mangle_name};
use diagnostic::DiagnosticLike;
use diagnostic::type_::TypeSymptom;
use internment::Intern;

use crate::Literal;
use crate::analysis::infer::{LayeredLocals, LocalTypes, TyInfer, TyInferEnv};
use crate::analysis::resolve::{is_type_surface, mangled_fn_call_name};
use crate::ty::Ty;

use super::call_args::check_call_args;
use super::unification::tys_equivalent;
use super::utils::{
    ImportSet, check_type_pattern_default_exprs, fmt_call_without_parens, is_field_of_type,
    suggest_typo_for_identifier, type_name_for_display,
};

pub(crate) struct UnknownRefChecker<'a, 'b, 'c> {
    pub ast: &'a crate::FileAst,
    pub tag_types: &'a HashMap<Intern<String>, Ty>,
    pub fn_return_types: &'a HashMap<Intern<String>, Ty>,
    pub variant_map: &'a crate::VariantMap,
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
            let is_method = self.fn_return_types.get(&mangled).is_some();
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
            if self.fn_return_types.get(&mangled).is_none() {
                let mangled_str = mangled.to_string();
                let suggestion = if !mangled_str.contains('.') {
                    suggest_typo_for_identifier(
                        self.tag_types,
                        self.fn_return_types,
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
            let call_env = crate::analysis::infer::TyInferEnv {
                tag_types: self.tag_types,
                fn_return_types: self.fn_return_types,
                locals: self.locals,
            };
            let (params, typevars) = self
                .ast
                .defs
                .get(&mangled)
                .map_or((vec![], HashMap::new()), |bind| {
                    (bind.resolved_params(), bind.resolved_typevars())
                });
            check_call_args(&params, &typevars, call, args, self.symptoms, &call_env);
        } else if call.path.segments.is_empty()
            && self.locals.get_type(&name).is_none()
            && self.fn_return_types.get(&mangled).is_none()
            && !self.imports.all.contains(&name)
        {
            let suggestion = suggest_typo_for_identifier(
                self.tag_types,
                self.fn_return_types,
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
            && self.fn_return_types.get(&mangled).is_none()
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

    fn visit_when_expr(&mut self, when: &crate::WhenExpr) -> ControlFlow<()> {
        let infer_env = TyInferEnv {
            tag_types: self.tag_types,
            fn_return_types: self.fn_return_types,
            locals: self.locals,
        };
        let subject_ty = when.subject.as_ref().map(|s| s.infer_ty(&infer_env));
        if let Some(subject) = &when.subject {
            let _ = self.visit_expr(subject);
        }

        let mut has_else = false;

        for arm in &when.arms {
            match arm {
                WhenArm::Cond {
                    condition, body, ..
                } => {
                    let _ = self.visit_expr(condition);

                    let cond_ty = condition.infer_ty(&infer_env);
                    let bool_ty = self.tag_types.get(&Intern::<String>::from_ref("Bool"));
                    let is_bool = bool_ty.is_some_and(|bt| tys_equivalent(&cond_ty, bt));
                    if !is_bool {
                        let got = type_name_for_display(&cond_ty);
                        self.symptoms.push(
                            TypeSymptom::ConditionNotBool { got }
                                .into_diagnostic(condition.span_id),
                        );
                    }

                    let _ = self.visit_expr(body);
                }
                WhenArm::Is { pattern, body, .. } => {
                    match &pattern.value {
                        TypeExpr::Literal(lit, _) => {
                            // Literal pattern in a const union match, e.g. `is 'debug'`
                            if let Some(Ty::ConstUnion {
                                name: union_name,
                                values,
                                ..
                            }) = &subject_ty
                            {
                                let display_name = const_union_variant_display_name(lit);
                                let in_set = values
                                    .iter()
                                    .any(|v| v.display_name().as_str() == display_name);
                                if !in_set {
                                    self.symptoms.push(
                                        TypeSymptom::NotAVariant {
                                            name: format!("{lit}"),
                                            union_name: union_name.to_string(),
                                        }
                                        .into_diagnostic(pattern.span_id),
                                    );
                                }
                            } else {
                                self.symptoms.push(
                                    TypeSymptom::UnknownTag {
                                        name: format!("{lit}"),
                                    }
                                    .into_diagnostic(pattern.span_id),
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
                                            .into_diagnostic(pattern.span_id),
                                        );
                                    }
                                }
                                _ => {
                                    if self.variant_map.get(&variant_name).is_none() {
                                        self.symptoms.push(
                                            TypeSymptom::UnknownTag {
                                                name: surface_name.to_string(),
                                            }
                                            .into_diagnostic(pattern.span_id),
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
                                .into_diagnostic(pattern.span_id),
                            );
                        }
                    }
                    let _ = self.visit_expr(body);
                }
                WhenArm::Else(body, _) => {
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
                    WhenArm::Cond { condition, .. } => condition.span_id,
                    WhenArm::Is { pattern, .. } => pattern.span_id,
                    WhenArm::Else(_, span) => *span,
                })
                .unwrap_or(crate::SpanId::INVALID);
            self.symptoms
                .push(TypeSymptom::MissingElseArm.into_diagnostic(span));
        }

        ControlFlow::Continue(())
    }

    fn visit_if_expr(&mut self, if_expr: &crate::IfExpr) -> ControlFlow<()> {
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
                if is_type_surface(&pattern.value) {
                    if let TypeExpr::Generic { params, .. } = &pattern.value {
                        for (k, _) in params.iter() {
                            if k.as_str() != "_" {
                                if_locals.insert(*k, Ty::Opaque(*k));
                            }
                        }
                    }
                    check_type_pattern_default_exprs(&pattern.value, &mut |e| {
                        let _ = self.visit_expr(e);
                    });
                } else {
                    self.symptoms.push(
                        TypeSymptom::UnknownTag {
                            name: "invalid is-pattern".to_string(),
                        }
                        .into_diagnostic(pattern.span_id),
                    );
                }
                for e in &if_expr.body {
                    let mut inner = UnknownRefChecker {
                        ast: self.ast,
                        tag_types: self.tag_types,
                        fn_return_types: self.fn_return_types,
                        variant_map: self.variant_map,
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

    fn visit_tag_call(&mut self, tc: &crate::TagCall) -> ControlFlow<()> {
        if let Some(path) = &tc.qual_path {
            if self.tag_types.get(&path.root).is_none() {
                self.symptoms.push(
                    TypeSymptom::UnknownTag {
                        name: path.root.to_string(),
                    }
                    .into_diagnostic(path.span_id()),
                );
            }
        } else if self.variant_map.get(&tc.name).is_none() {
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
                if self.variant_map.get(name).is_none() {
                    self.symptoms.push(
                        TypeSymptom::UnknownTag {
                            name: name.to_string(),
                        }
                        .into_diagnostic(*span),
                    );
                }
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

/// Convert a `Literal` to the variant display name used by `ConstValue`.
fn const_union_variant_display_name(lit: &Literal) -> String {
    match lit {
        Literal::String(s) => s.clone(),
        Literal::Int(n) => n.to_string(),
        Literal::Float(f) => f.to_string(),
        Literal::Number(n) => n.to_string(),
    }
}
