//! AST validation — unknown-reference checking and unused-binding detection.

use ast::{
    Bind, BindValue, Expr, FileAst, FormatPart, HasSpanId, IfCondition, Loop,
    ParameterKind, Spanned, WhenArm, type_surface_mangle_name,
};
use internment::Intern;
use std::collections::{HashMap, HashSet};

use crate::env::TyEnv;
use crate::resolve::{is_type_surface, mangled_fn_call_name};
use crate::ty::Ty;
use crate::{LayeredLocals, LocalTypes, TyInfer, TyInferEnv};

impl TyEnv {
    pub fn check_unknowns(&self, ast: &FileAst, symptoms: &mut Vec<diagnostic::Diagnostic>) {
        for bind in ast.defs.values() {
            if !bind.attributes().matches_current_platform() {
                continue;
            }
            let mut locals = HashMap::new();
            if let Some(params) = bind.params() {
                for (name, kind) in params.iter() {
                    locals.insert(*name, self.resolve_parameter_kind(kind));
                }
            }
            self.check_bind(bind, symptoms, &locals);
        }
    }

    fn check_bind(
        &self,
        bind: &Bind,
        symptoms: &mut Vec<diagnostic::Diagnostic>,
        locals: &dyn LocalTypes,
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
                check_return_variants(bind, &valid_variants, *union_name, symptoms);
            }
        }
        match bind.value() {
            BindValue::Expr(expr) => self.check_expr(expr, symptoms, locals),
            BindValue::Body { exprs, ret } => {
                use diagnostic::DiagnosticLike;
                use diagnostic::type_::TypeSymptom;

                let mut body_locals = LayeredLocals::new(locals);
                for expr in exprs.iter() {
                    if let Expr::Bind(inner) = &**expr {
                        self.check_bind(inner, symptoms, &body_locals);
                        body_locals.insert(inner.name(), {
                            let env = TyInferEnv {
                                tag_types: &self.tag_types,
                                fn_return_types: &self.fn_return_types,
                                locals: &HashMap::new(),
                            };
                            inner.infer_ty(&env)
                        });
                    } else {
                        self.check_expr(expr, symptoms, &body_locals);
                    }
                }
                if let Some(ret_expr) = &ret.0 {
                    self.check_expr(ret_expr, symptoms, &body_locals);
                }

                let mut suffix_refs: HashSet<Intern<String>> = HashSet::new();
                if let Some(e) = &ret.0 {
                    collect_referenced_names(e, &mut suffix_refs);
                }
                let mut unused_spans: Vec<_> = Vec::new();
                for expr in exprs.iter().rev() {
                    if let Expr::Bind(inner) = &**expr {
                        let name = inner.name();
                        if !suffix_refs.contains(&name) && !name.starts_with('_') {
                            unused_spans.push((name, inner.name_span));
                        }
                        collect_bind_value_refs(inner, &mut suffix_refs);
                    } else {
                        collect_referenced_names(expr, &mut suffix_refs);
                    }
                }
                for (name, span) in unused_spans.into_iter().rev() {
                    symptoms.push(
                        TypeSymptom::UnusedBinding {
                            name: name.to_string(),
                        }
                        .into_diagnostic(span),
                    );
                }
            }
            BindValue::Extern => {}
        }
    }

    fn check_expr(
        &self,
        expr: &Expr,
        symptoms: &mut Vec<diagnostic::Diagnostic>,
        locals: &dyn LocalTypes,
    ) {
        use diagnostic::DiagnosticLike;
        use diagnostic::type_::TypeSymptom;

        match expr {
            Expr::FnCall(call) => {
                let name = call.path.root;
                let mangled = mangled_fn_call_name(call);
                if let Some(args) = &call.args {
                    if self.fn_return_ty(&mangled).is_none() {
                        symptoms.push(
                            TypeSymptom::UnknownBinding {
                                name: mangled.to_string(),
                            }
                            .into_diagnostic(call.path.span_id()),
                        );
                    }
                    for arg in args {
                        self.check_expr(arg, symptoms, locals);
                    }
                } else if call.path.segments.is_empty()
                    && locals.get_type(&name).is_none()
                    && self.fn_return_ty(&mangled).is_none()
                {
                    symptoms.push(
                        TypeSymptom::UnknownBinding {
                            name: mangled.to_string(),
                        }
                        .into_diagnostic(call.path.span_id()),
                    );
                }
            }
            Expr::Bind(bind) => self.check_bind(bind, symptoms, locals),
            Expr::Binary(bin) => {
                self.check_expr(&bin.lhs, symptoms, locals);
                self.check_expr(&bin.rhs, symptoms, locals);
            }
            Expr::When(w) => {
                let subject_ty = w
                    .subject
                    .as_ref()
                    .map(|s| s.infer_ty(&self.infer_env(locals)));
                if let Some(subject) = &w.subject {
                    self.check_expr(subject, symptoms, locals);
                }
                for arm in &w.arms {
                    match arm {
                        WhenArm::Cond { condition, body } => {
                            self.check_expr(condition, symptoms, locals);
                            self.check_expr(body, symptoms, locals);
                        }
                        WhenArm::Is { pattern, body } => {
                            if is_type_surface(&pattern.0) {
                                let surface_name = type_surface_mangle_name(&pattern.0);
                                let variant_name = Intern::<String>::from_ref(surface_name);
                                match &subject_ty {
                                    Some(Ty::Union {
                                        name: union_name,
                                        variants,
                                    }) => {
                                        if !variants.iter().any(|(vname, _)| vname == &variant_name)
                                        {
                                            symptoms.push(
                                                TypeSymptom::NotAVariant {
                                                    name: surface_name.to_string(),
                                                    union_name: union_name.to_string(),
                                                }
                                                .into_diagnostic(pattern.1),
                                            );
                                        }
                                    }
                                    _ => {
                                        if self.lookup_variant(variant_name).is_none() {
                                            symptoms.push(
                                                TypeSymptom::UnknownTag {
                                                    name: surface_name.to_string(),
                                                }
                                                .into_diagnostic(pattern.1),
                                            );
                                        }
                                    }
                                }
                                check_type_pattern_default_exprs(&pattern.0, &mut |e| {
                                    self.check_expr(e, symptoms, locals);
                                });
                            } else {
                                symptoms.push(
                                    TypeSymptom::UnknownTag {
                                        name: "invalid is-pattern".to_string(),
                                    }
                                    .into_diagnostic(pattern.1),
                                );
                            }
                            self.check_expr(body, symptoms, locals);
                        }
                        WhenArm::Else(body) => {
                            self.check_expr(body, symptoms, locals);
                        }
                    }
                }
            }
            Expr::If(if_expr) => match &if_expr.condition {
                IfCondition::Bool(cond) => {
                    self.check_expr(cond, symptoms, locals);
                    for e in &if_expr.body {
                        self.check_expr(e, symptoms, locals);
                    }
                }
                IfCondition::Pattern { subject, pattern } => {
                    self.check_expr(subject, symptoms, locals);
                    let mut if_locals = LayeredLocals::new(locals);
                    if is_type_surface(&pattern.0) {
                        if let Expr::TypeGeneric { params, .. } = &pattern.0 {
                            for (k, _) in params.iter() {
                                if k.as_str() != "_" {
                                    if_locals.insert(*k, Ty::Opaque(*k));
                                }
                            }
                        }
                        check_type_pattern_default_exprs(&pattern.0, &mut |e| {
                            self.check_expr(e, symptoms, locals);
                        });
                    } else {
                        symptoms.push(
                            TypeSymptom::UnknownTag {
                                name: "invalid is-pattern".to_string(),
                            }
                            .into_diagnostic(pattern.1),
                        );
                    }
                    for e in &if_expr.body {
                        self.check_expr(e, symptoms, &if_locals);
                    }
                }
            },
            Expr::Loop(loop_expr) => match loop_expr {
                Loop::While(w) => {
                    self.check_expr(&w.cond, symptoms, locals);
                    for e in &w.exprs {
                        self.check_expr(e, symptoms, locals);
                    }
                }
                Loop::ForIn(f) => {
                    self.check_expr(&f.iter, symptoms, locals);
                    for e in &f.exprs {
                        self.check_expr(e, symptoms, locals);
                    }
                }
            },
            Expr::TupleLit(elems) => {
                for e in elems {
                    self.check_expr(e, symptoms, locals);
                }
            }
            Expr::TupleAlloc { init, .. } => self.check_expr(init, symptoms, locals),
            Expr::TupleGet { base, .. } => self.check_expr(base, symptoms, locals),
            Expr::TupleSet { base, value, .. } => {
                self.check_expr(base, symptoms, locals);
                self.check_expr(value, symptoms, locals);
            }
            Expr::BufGet { buf, index, .. } => {
                self.check_expr(buf, symptoms, locals);
                self.check_expr(index, symptoms, locals);
            }
            Expr::BufSet {
                buf, index, value, ..
            } => {
                self.check_expr(buf, symptoms, locals);
                self.check_expr(index, symptoms, locals);
                self.check_expr(value, symptoms, locals);
            }
            Expr::Cast { expr, .. } => self.check_expr(expr, symptoms, locals),
            Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => {
                self.check_expr(e, symptoms, locals);
            }
            Expr::AnonymousTag(name, span) => {
                if self.lookup_variant(*name).is_none() {
                    symptoms.push(
                        TypeSymptom::UnknownTag {
                            name: name.to_string(),
                        }
                        .into_diagnostic(*span),
                    );
                }
            }
            Expr::TagCall(tc) => {
                if let Some(path) = &tc.qual_path {
                    if self.lookup_tag(path.root).is_none() {
                        symptoms.push(
                            TypeSymptom::UnknownTag {
                                name: path.root.to_string(),
                            }
                            .into_diagnostic(path.span_id()),
                        );
                    }
                } else if self.lookup_variant(tc.name).is_none() {
                    symptoms.push(
                        TypeSymptom::UnknownTag {
                            name: tc.name.to_string(),
                        }
                        .into_diagnostic(tc.span_id()),
                    );
                }
                for arg in &tc.args {
                    self.check_expr(arg, symptoms, locals);
                }
            }
            Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. } => {
                self.check_type_expr(expr, symptoms);
            }
            Expr::Lit(_)
            | Expr::SelfRef(_)
            | Expr::Range(_)
            | Expr::FormatString(_)
            | Expr::Asm(_) => {}
        }
    }

    fn check_type_expr(&self, e: &Expr, symptoms: &mut Vec<diagnostic::Diagnostic>) {
        use diagnostic::DiagnosticLike;
        use diagnostic::type_::TypeSymptom;

        match e {
            Expr::TypeNominal(name, span) if self.lookup_tag(*name).is_none() => {
                symptoms.push(
                    TypeSymptom::UnknownTag {
                        name: name.to_string(),
                    }
                    .into_diagnostic(*span),
                );
            }
            Expr::TypeGeneric { name, params, span } => {
                if self.lookup_tag(*name).is_none() {
                    symptoms.push(
                        TypeSymptom::UnknownTag {
                            name: name.to_string(),
                        }
                        .into_diagnostic(*span),
                    );
                }
                for (_, kind) in params {
                    if let ParameterKind::Tagged(sp) = kind
                        && is_type_surface(&sp.0)
                    {
                        self.check_type_expr(&sp.0, symptoms);
                    }
                }
            }
            Expr::TypeQualified(path) if self.lookup_tag(path.root).is_none() => {
                symptoms.push(
                    TypeSymptom::UnknownTag {
                        name: path.root.to_string(),
                    }
                    .into_diagnostic(path.span_id()),
                );
            }
            _ => {}
        }
    }
}

fn check_type_pattern_default_exprs(surface: &Expr, check: &mut impl FnMut(&Spanned<Expr>)) {
    let Expr::TypeGeneric { params, .. } = surface else {
        return;
    };
    for (_, pk) in params {
        match pk {
            ParameterKind::Default(e) => check(e),
            ParameterKind::Tagged(sp) => check_type_surface_defaults(&sp.0, check),
            ParameterKind::Generic => {}
        }
    }
}

fn check_type_surface_defaults(e: &Expr, check: &mut impl FnMut(&Spanned<Expr>)) {
    if let Expr::TypeGeneric { params, .. } = e {
        for (_, pk) in params {
            match pk {
                ParameterKind::Default(e) => check(e),
                ParameterKind::Tagged(sp) => check_type_surface_defaults(&sp.0, check),
                ParameterKind::Generic => {}
            }
        }
    }
}

fn collect_referenced_names(expr: &Expr, out: &mut HashSet<Intern<String>>) {
    match expr {
        Expr::FnCall(call) => {
            if call.path.segments.is_empty() {
                out.insert(call.path.root);
            }
            if let Some(args) = &call.args {
                for a in args {
                    collect_referenced_names(a, out);
                }
            }
        }
        Expr::Bind(bind) => collect_bind_value_refs(bind, out),
        Expr::Binary(bin) => {
            collect_referenced_names(&bin.lhs, out);
            collect_referenced_names(&bin.rhs, out);
        }
        Expr::When(w) => {
            if let Some(s) = &w.subject {
                collect_referenced_names(s, out);
            }
            for arm in &w.arms {
                match arm {
                    WhenArm::Cond { condition, body } => {
                        collect_referenced_names(condition, out);
                        collect_referenced_names(body, out);
                    }
                    WhenArm::Is { pattern, body } => {
                        collect_type_pattern_refs(&pattern.0, out);
                        collect_referenced_names(&body.0, out);
                    }
                    WhenArm::Else(body) => collect_referenced_names(body, out),
                }
            }
        }
        Expr::If(if_expr) => {
            match &if_expr.condition {
                IfCondition::Bool(c) => collect_referenced_names(c, out),
                IfCondition::Pattern { subject, pattern } => {
                    collect_referenced_names(subject, out);
                    collect_type_pattern_refs(&pattern.0, out);
                }
            }
            for e in &if_expr.body {
                collect_referenced_names(e, out);
            }
        }
        Expr::Loop(loop_expr) => match loop_expr {
            Loop::While(w) => {
                collect_referenced_names(&w.cond, out);
                for e in &w.exprs {
                    collect_referenced_names(e, out);
                }
            }
            Loop::ForIn(f) => {
                collect_referenced_names(&f.iter, out);
                for e in &f.exprs {
                    collect_referenced_names(e, out);
                }
            }
        },
        Expr::TupleLit(elems) => {
            for e in elems {
                collect_referenced_names(e, out);
            }
        }
        Expr::TupleAlloc { init, .. } => collect_referenced_names(init, out),
        Expr::TupleGet { base, .. } => collect_referenced_names(base, out),
        Expr::TupleSet { base, value, .. } => {
            collect_referenced_names(base, out);
            collect_referenced_names(value, out);
        }
        Expr::BufGet { buf, index, .. } => {
            collect_referenced_names(buf, out);
            collect_referenced_names(index, out);
        }
        Expr::BufSet {
            buf, index, value, ..
        } => {
            collect_referenced_names(buf, out);
            collect_referenced_names(index, out);
            collect_referenced_names(value, out);
        }
        Expr::Cast { expr, .. } => collect_referenced_names(expr, out),
        Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => {
            collect_referenced_names(e, out);
        }
        Expr::FormatString(fs) => {
            for p in &fs.parts {
                if let FormatPart::Expr(e) = p {
                    collect_referenced_names(e, out);
                }
            }
        }
        Expr::Range(range) => {
            collect_referenced_names(&range.start, out);
            collect_referenced_names(&range.end, out);
        }
        Expr::TypeGeneric { .. } => collect_type_surface_refs(expr, out),
        Expr::Lit(_)
        | Expr::SelfRef(_)
        | Expr::AnonymousTag(..)
        | Expr::TagCall(_)
        | Expr::Asm(_)
        | Expr::TypeNominal(..)
        | Expr::TypeQualified(_) => {}
    }
}

fn collect_bind_value_refs(bind: &Bind, out: &mut HashSet<Intern<String>>) {
    match bind.value() {
        BindValue::Expr(e) => collect_referenced_names(e, out),
        BindValue::Body { exprs, ret } => {
            for e in exprs {
                collect_referenced_names(e, out);
            }
            if let Some(e) = &ret.0 {
                collect_referenced_names(e, out);
            }
        }
        BindValue::Extern => {}
    }
}

fn collect_type_pattern_refs(surface: &Expr, out: &mut HashSet<Intern<String>>) {
    let Expr::TypeGeneric { params, .. } = surface else {
        return;
    };
    for (_, pk) in params {
        match pk {
            ParameterKind::Default(e) => collect_referenced_names(&e.0, out),
            ParameterKind::Tagged(sp) => collect_type_surface_refs(&sp.0, out),
            ParameterKind::Generic => {}
        }
    }
}

fn collect_type_surface_refs(e: &Expr, out: &mut HashSet<Intern<String>>) {
    if let Expr::TypeGeneric { params, .. } = e {
        for (_, pk) in params {
            match pk {
                ParameterKind::Default(e) => collect_referenced_names(&e.0, out),
                ParameterKind::Tagged(sp) => collect_type_surface_refs(&sp.0, out),
                ParameterKind::Generic => {}
            }
        }
    }
}

fn check_return_variants(
    bind: &Bind,
    valid_variants: &[Intern<String>],
    union_name: Intern<String>,
    symptoms: &mut Vec<diagnostic::Diagnostic>,
) {
    use diagnostic::DiagnosticLike;
    use diagnostic::type_::TypeSymptom;

    fn check_expr(
        expr: &Spanned<Expr>,
        valid_variants: &[Intern<String>],
        union_name: Intern<String>,
        symptoms: &mut Vec<diagnostic::Diagnostic>,
    ) {
        match &expr.0 {
            Expr::AnonymousTag(name, span)
                if !valid_variants.iter().any(|v| v.as_str() == name.as_str()) =>
            {
                symptoms.push(
                    TypeSymptom::NotAVariant {
                        name: name.to_string(),
                        union_name: union_name.to_string(),
                    }
                    .into_diagnostic(*span),
                );
            }
            Expr::TagCall(tc)
                if !valid_variants
                    .iter()
                    .any(|v| v.as_str() == tc.name.as_str()) =>
            {
                symptoms.push(
                    TypeSymptom::NotAVariant {
                        name: tc.name.to_string(),
                        union_name: union_name.to_string(),
                    }
                    .into_diagnostic(tc.span_id()),
                );
            }
            Expr::If(if_expr) => {
                for e in &if_expr.body {
                    check_expr(e, valid_variants, union_name, symptoms);
                }
                if let Some(ret_expr) = &if_expr.ret.0 {
                    check_expr(ret_expr, valid_variants, union_name, symptoms);
                } else {
                    symptoms.push(
                        TypeSymptom::EmptyReturn {
                            expected_type: union_name.to_string(),
                        }
                        .into_diagnostic(expr.1),
                    );
                }
            }
            Expr::When(w) => {
                for arm in &w.arms {
                    match arm {
                        WhenArm::Cond { body, .. } => {
                            check_expr(body, valid_variants, union_name, symptoms)
                        }
                        WhenArm::Is { body, .. } => {
                            check_expr(body, valid_variants, union_name, symptoms)
                        }
                        WhenArm::Else(body) => {
                            check_expr(body, valid_variants, union_name, symptoms)
                        }
                    }
                }
            }
            Expr::Bind(inner) => match inner.value() {
                BindValue::Expr(e) => check_expr(e, valid_variants, union_name, symptoms),
                BindValue::Body { exprs, ret } => {
                    for e in exprs {
                        check_expr(e, valid_variants, union_name, symptoms);
                    }
                    if let Some(r) = &ret.0 {
                        check_expr(r, valid_variants, union_name, symptoms);
                    } else {
                        symptoms.push(
                            TypeSymptom::EmptyReturn {
                                expected_type: union_name.to_string(),
                            }
                            .into_diagnostic(inner.name_span),
                        );
                    }
                }
                BindValue::Extern => {}
            },
            _ => {}
        }
    }

    match bind.value() {
        BindValue::Expr(expr) => check_expr(expr, valid_variants, union_name, symptoms),
        BindValue::Body { exprs, ret } => {
            for expr in exprs {
                check_expr(expr, valid_variants, union_name, symptoms);
            }
            if let Some(ret_expr) = &ret.0 {
                check_expr(ret_expr, valid_variants, union_name, symptoms);
            } else {
                symptoms.push(
                    TypeSymptom::EmptyReturn {
                        expected_type: union_name.to_string(),
                    }
                    .into_diagnostic(bind.name_span),
                );
            }
        }
        BindValue::Extern => {}
    }
}
