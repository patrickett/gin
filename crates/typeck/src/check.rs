//! AST validation — unknown-reference checking and unused-binding detection.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use ast::visit::{Visitor, walk_bind_value, walk_expr, walk_fn_call};
use ast::{
    Bind, BindValue, Expr, FileAst, HasSpanId, IfCondition, ParameterKind, SpanId, Spanned,
    WhenArm, type_surface_mangle_name,
};
use diagnostic::DiagnosticLike;
use diagnostic::type_::TypeSymptom;
use internment::Intern;

use crate::env::TyEnv;
use crate::infer::LayeredLocals;
use crate::resolve::{is_type_surface, mangled_fn_call_name};
use crate::ty::Ty;
use crate::{LocalTypes, TyInfer, TyInferEnv};

use ControlFlow::Continue;

/// Import information for the current file.
struct ImportSet {
    all: HashSet<Intern<String>>,
    bundle_members: HashSet<Intern<String>>,
    module_prefixes: HashSet<Intern<String>>,
    alias_spans: HashSet<SpanId>,
}

/// Collect import names and bundle member names from the AST.
fn collect_import_names(ast: &FileAst) -> ImportSet {
    let mut all = HashSet::new();
    let mut bundle_members = HashSet::new();
    let mut module_prefixes = HashSet::new();
    let alias_spans: HashSet<SpanId> = ast.symbol_alias_spans.iter().copied().collect();
    for imp in ast.uses() {
        for mi in &imp.0 {
            let name = mi
                .alias
                .unwrap_or_else(|| Intern::<String>::new(mi.effective_name()));
            all.insert(name);
            if let ast::ImportSource::LocalBundle(b) = &mi.source {
                for member in &b.members {
                    let member_name = member.alias.unwrap_or(member.export);
                    all.insert(member_name);
                    bundle_members.insert(member_name);
                }
            } else if let ast::ImportSource::Package(mp) = &mi.source
                && !mp.segments.is_empty()
            {
                bundle_members.insert(name);
            }
            if let ast::ImportSource::Package(mp) = &mi.source
                && mp.segments.is_empty()
            {
                let prefix = mi.alias.unwrap_or(mp.root);
                module_prefixes.insert(prefix);
            }
            if let ast::ImportSource::Local(_, _) = &mi.source
                && let Some(alias) = mi.alias
            {
                module_prefixes.insert(alias);
            }
        }
    }
    ImportSet {
        all,
        bundle_members,
        module_prefixes,
        alias_spans,
    }
}

/// Levenshtein distance for short identifiers (imports, bind names).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let n = a.len();
    let m = b.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0usize; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

impl TyEnv {
    fn suggest_typo_for_identifier(
        &self,
        unknown: &str,
        imports: &HashSet<Intern<String>>,
    ) -> Option<String> {
        if unknown.is_empty() || unknown.contains('.') {
            return None;
        }
        let mut seen: HashSet<String> = HashSet::new();
        let mut candidates: Vec<String> = Vec::new();
        for k in imports {
            let s = k.to_string();
            if seen.insert(s.clone()) {
                candidates.push(s);
            }
        }
        for k in self.fn_return_types.keys() {
            let s = k.to_string();
            if seen.insert(s.clone()) {
                candidates.push(s);
            }
        }
        for k in self.tag_types.keys() {
            let s = k.to_string();
            if seen.insert(s.clone()) {
                candidates.push(s);
            }
        }
        let mut scored: Vec<(usize, String)> = candidates
            .into_iter()
            .filter(|c| c != unknown)
            .map(|c| (levenshtein(unknown, &c), c))
            .filter(|(d, _)| *d > 0 && *d <= 2)
            .collect();
        if scored.is_empty() {
            return None;
        }
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let best_d = scored[0].0;
        if scored.len() > 1 && scored[1].0 == best_d {
            return None;
        }
        Some(scored[0].1.clone())
    }

    pub fn check_unknowns(&self, ast: &FileAst, symptoms: &mut Vec<diagnostic::Diagnostic>) {
        let imports = collect_import_names(ast);
        for bind in ast.defs.values() {
            if !bind.attributes().matches_current_platform() {
                continue;
            }
            let subst = bind
                .receiver_type_surface()
                .map(|sp| crate::resolve::typevars_from_receiver(&sp.0))
                .unwrap_or_default();
            let mut locals = HashMap::new();
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
                check_return_variants(bind, &valid_variants, *union_name, symptoms);
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
                let mut body_locals = LayeredLocals::new(locals);
                for expr in exprs.iter() {
                    if let Expr::Bind(inner) = &**expr {
                        self.check_bind(inner, symptoms, &body_locals, imports);
                        body_locals.insert(inner.name(), {
                            let env = TyInferEnv {
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

                let mut suffix_refs: HashSet<Intern<String>> = HashSet::new();
                if let Some(e) = &ret.0 {
                    let mut collector = RefCollector {
                        refs: &mut suffix_refs,
                    };
                    let _ = walk_expr(&mut collector, e);
                }
                let mut unused_spans: Vec<_> = Vec::new();
                for expr in exprs.iter().rev() {
                    if let Expr::Bind(inner) = &**expr {
                        let name = inner.name();
                        if !suffix_refs.contains(&name) && !name.starts_with('_') {
                            unused_spans.push((name, inner.name_span));
                        }
                        let mut collector = RefCollector {
                            refs: &mut suffix_refs,
                        };
                        let _ = walk_bind_value(&mut collector, inner.value());
                    } else {
                        let mut collector = RefCollector {
                            refs: &mut suffix_refs,
                        };
                        let _ = walk_expr(&mut collector, expr);
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

    fn check_type_expr(&self, e: &Expr, symptoms: &mut Vec<diagnostic::Diagnostic>) {
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

    fn check_body_matches_return(
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
            .map(|sp| crate::resolve::typevars_from_receiver(&sp.0))
            .unwrap_or_default();
        let expected =
            crate::resolve::resolve_type_expr_with_subst(&return_tag.0, &self.tag_types, &subst);

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
            symptoms.push(TypeSymptom::Mismatch.into_diagnostic(span));
        }
    }

    fn check_call_args(
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

struct UnknownRefChecker<'a, 'b, 'c> {
    ty_env: &'a TyEnv,
    symptoms: &'b mut Vec<diagnostic::Diagnostic>,
    locals: &'c dyn LocalTypes,
    imports: &'a ImportSet,
}

impl Visitor for UnknownRefChecker<'_, '_, '_> {
    fn visit_fn_call(&mut self, call: &ast::FnCall) -> ControlFlow<()> {
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
                return Continue(());
            }
        }
        let name = call.path.root;
        let mangled = mangled_fn_call_name(call);
        if let Some(args) = &call.args {
            if self.ty_env.fn_return_ty(&mangled).is_none() {
                let mangled_str = mangled.to_string();
                let suggestion = if !mangled_str.contains('.') {
                    self.ty_env
                        .suggest_typo_for_identifier(&mangled_str, &self.imports.all)
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
            let suggestion = self
                .ty_env
                .suggest_typo_for_identifier(name.as_str(), &self.imports.all);
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
        Continue(())
    }

    fn visit_when_expr(&mut self, when: &ast::WhenExpr) -> ControlFlow<()> {
        let subject_ty = when
            .subject
            .as_ref()
            .map(|s| s.infer_ty(&self.ty_env.infer_env(self.locals)));
        if let Some(subject) = &when.subject {
            let _ = self.visit_expr(subject);
        }
        for arm in &when.arms {
            match arm {
                WhenArm::Cond { condition, body } => {
                    let _ = self.visit_expr(condition);
                    let _ = self.visit_expr(body);
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
                    let _ = self.visit_expr(body);
                }
                WhenArm::Else(body) => {
                    let _ = self.visit_expr(body);
                }
            }
        }
        Continue(())
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
                    // Use a new checker with the enriched if_locals
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
        Continue(())
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
        Continue(())
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
                Continue(())
            }
            Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. } => {
                self.ty_env.check_type_expr(expr, self.symptoms);
                Continue(())
            }
            Expr::Lit(_)
            | Expr::SelfRef(_)
            | Expr::Range(_)
            | Expr::FormatString(_)
            | Expr::Asm(_) => Continue(()),
            _ => walk_expr(self, expr),
        }
    }
}

/// Check whether `segment` names a field of the given type.
fn is_field_of_type(ty: &Ty, segment: &Intern<String>) -> bool {
    match ty {
        Ty::Record { fields, .. } => fields.iter().any(|(name, _)| name == segment),
        Ty::Ptr { inner } | Ty::Ref { inner } if inner.is_record() => {
            if let Ty::Record { fields, .. } = inner.as_ref() {
                fields.iter().any(|(name, _)| name == segment)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn fmt_call_without_parens(call: &ast::FnCall) -> String {
    if call.path.segments.is_empty() {
        call.path.root.as_str().to_string()
    } else {
        let segs: Vec<&str> = call.path.segments.iter().map(|s| s.as_str()).collect();
        format!("{}.{}", call.path.root.as_str(), segs.join("."))
    }
}

/// One-way structural unification check between an actual type and an expected
/// type, with type-variable bindings collected in `bindings`.
pub(crate) fn ty_unifies_with(
    actual: &Ty,
    expected: &Ty,
    bindings: &mut HashMap<Intern<String>, Ty>,
) -> bool {
    if tys_equivalent(actual, expected) {
        return true;
    }
    match (actual, expected) {
        (_, Ty::Opaque(name)) => {
            let is_unbound = match bindings.get(name) {
                None => true,
                Some(Ty::Opaque(prev)) if prev == name => true,
                _ => false,
            };
            if is_unbound {
                bindings.insert(*name, strip_literal(actual));
                return true;
            }
            bindings
                .get(name)
                .map(|prev| tys_equivalent(prev, actual))
                .unwrap_or(false)
        }
        (Ty::Opaque(_), _) => true,
        (Ty::Tuple(elems), Ty::Record { fields, .. }) => {
            if elems.len() != fields.len() {
                return false;
            }
            elems
                .iter()
                .zip(fields.iter())
                .all(|(e, (_, f))| ty_unifies_with(e, f, bindings))
        }
        (Ty::Record { fields, .. }, Ty::Tuple(elems)) => {
            if fields.len() != elems.len() {
                return false;
            }
            fields
                .iter()
                .zip(elems.iter())
                .all(|((_, f), e)| ty_unifies_with(f, e, bindings))
        }
        (Ty::Tuple(a), Ty::Tuple(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| ty_unifies_with(x, y, bindings))
        }
        (
            Ty::Record {
                fields: af,
                name: an,
            },
            Ty::Record {
                fields: bf,
                name: bn,
            },
        ) => {
            an == bn
                && af.len() == bf.len()
                && af
                    .iter()
                    .zip(bf.iter())
                    .all(|((_, a), (_, b))| ty_unifies_with(a, b, bindings))
        }
        (Ty::Ptr { inner: a }, Ty::Ptr { inner: b })
        | (Ty::Ref { inner: a }, Ty::Ref { inner: b }) => ty_unifies_with(a, b, bindings),
        _ => false,
    }
}

/// Structural type equivalence ignoring literal `value` fields on Int/Float.
fn tys_equivalent(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (
            Ty::Int {
                width: aw,
                signed: as_,
                ..
            },
            Ty::Int {
                width: bw,
                signed: bs,
                ..
            },
        ) => aw == bw && as_ == bs,
        (Ty::Float { .. }, Ty::Float { .. }) => true,
        (Ty::Tuple(av), Ty::Tuple(bv)) => {
            av.len() == bv.len() && av.iter().zip(bv.iter()).all(|(x, y)| tys_equivalent(x, y))
        }
        (
            Ty::Record {
                name: an,
                fields: af,
            },
            Ty::Record {
                name: bn,
                fields: bf,
            },
        ) => {
            an == bn
                && af.len() == bf.len()
                && af
                    .iter()
                    .zip(bf.iter())
                    .all(|((_, x), (_, y))| tys_equivalent(x, y))
        }
        (Ty::Ptr { inner: a }, Ty::Ptr { inner: b })
        | (Ty::Ref { inner: a }, Ty::Ref { inner: b }) => tys_equivalent(a, b),
        _ => a == b,
    }
}

fn strip_literal(ty: &Ty) -> Ty {
    match ty {
        Ty::Int { width, signed, .. } => Ty::Int {
            width: *width,
            signed: *signed,
            value: None,
        },
        Ty::Float { .. } => Ty::Float { value: None },
        _ => ty.clone(),
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

struct RefCollector<'a> {
    refs: &'a mut HashSet<Intern<String>>,
}

impl Visitor for RefCollector<'_> {
    fn visit_fn_call(&mut self, call: &ast::FnCall) -> ControlFlow<()> {
        self.refs.insert(call.path.root);
        walk_fn_call(self, call)
    }
}

fn check_return_variants(
    bind: &Bind,
    valid_variants: &[Intern<String>],
    union_name: Intern<String>,
    symptoms: &mut Vec<diagnostic::Diagnostic>,
) {
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
