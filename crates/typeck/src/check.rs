//! AST validation — unknown-reference checking and unused-binding detection.

use ast::{
    Bind, BindValue, Expr, FileAst, FormatPart, HasSpanId, IfCondition, Loop, ParameterKind,
    Spanned, WhenArm, type_surface_mangle_name,
};
use internment::Intern;
use std::collections::{HashMap, HashSet};

use crate::env::TyEnv;
use crate::resolve::{is_type_surface, mangled_fn_call_name};
use crate::ty::Ty;
use crate::{LayeredLocals, LocalTypes, TyInfer, TyInferEnv};

/// Value-level names brought into scope by `use` (package root, last segment, or `as` alias).
fn collect_import_names(ast: &FileAst) -> HashSet<Intern<String>> {
    let mut out = HashSet::new();
    for imp in ast.uses() {
        for mi in &imp.0 {
            let name = mi
                .alias
                .unwrap_or_else(|| Intern::<String>::new(mi.effective_name()));
            out.insert(name);
        }
    }
    out
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
    /// Best single-character-edit match among imports, top-level functions, and declared tags.
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
            // Method-scoped type variables introduced by a generic receiver
            // (e.g. `x` in `Range(x).new`). Treated as opaque while checking
            // this method's own body.
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
            // `self` resolves against the same subst so methods can reference
            // their own receiver's record fields with the type variable bound.
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
        imports: &HashSet<Intern<String>>,
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
        // Body-vs-return-tag tuple-IS-record check: when the return tag is a
        // record (e.g. `Range(x)`) and the body produces a tuple of matching
        // arity (e.g. `(start, end)`), accept it without requiring an explicit
        // record-literal syntax. This is the first step toward fully unifying
        // `Ty::Tuple` and `Ty::Record` per the design intent.
        self.check_body_matches_return(bind, symptoms, locals);
        match bind.value() {
            BindValue::Expr(expr) => self.check_expr(expr, symptoms, locals, imports),
            BindValue::Body { exprs, ret } => {
                use diagnostic::DiagnosticLike;
                use diagnostic::type_::TypeSymptom;

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
                        self.check_expr(expr, symptoms, &body_locals, imports);
                    }
                }
                if let Some(ret_expr) = &ret.0 {
                    self.check_expr(ret_expr, symptoms, &body_locals, imports);
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
        imports: &HashSet<Intern<String>>,
    ) {
        use diagnostic::DiagnosticLike;
        use diagnostic::type_::TypeSymptom;

        match expr {
            Expr::FnCall(call) => {
                let name = call.path.root;
                let mangled = mangled_fn_call_name(call);
                if let Some(args) = &call.args {
                    if self.fn_return_ty(&mangled).is_none() {
                        let mangled_str = mangled.to_string();
                        let suggestion = if !mangled_str.contains('.') {
                            self.suggest_typo_for_identifier(&mangled_str, imports)
                        } else {
                            None
                        };
                        symptoms.push(
                            TypeSymptom::UnknownBinding {
                                name: mangled_str,
                                did_you_mean: suggestion,
                            }
                            .into_diagnostic(call.path.span_id()),
                        );
                    }
                    for arg in args {
                        self.check_expr(arg, symptoms, locals, imports);
                    }
                    // Argument-vs-parameter unification: if the called function
                    // has a known param signature, walk the args and check each
                    // one against the corresponding param type. Type-variable
                    // unification (`x` shared across `start x, end x`) is what
                    // catches `Range.new(1, "hi")`.
                    self.check_call_args(&mangled, call, args, symptoms, locals);
                } else if call.path.segments.is_empty()
                    && locals.get_type(&name).is_none()
                    && self.fn_return_ty(&mangled).is_none()
                    && !imports.contains(&name)
                {
                    let suggestion = self.suggest_typo_for_identifier(name.as_str(), imports);
                    symptoms.push(
                        TypeSymptom::UnknownBinding {
                            name: mangled.to_string(),
                            did_you_mean: suggestion,
                        }
                        .into_diagnostic(call.path.span_id()),
                    );
                } else if call.path.segments.is_empty()
                    && call.args.is_none()
                    && locals.get_type(&name).is_none()
                    && self.fn_return_ty(&mangled).is_none()
                    && imports.contains(&name)
                {
                    symptoms.push(
                        TypeSymptom::NotExpr {
                            name: name.to_string(),
                        }
                        .into_diagnostic(call.path.span_id()),
                    );
                }
            }
            Expr::Bind(bind) => self.check_bind(bind, symptoms, locals, imports),
            Expr::Binary(bin) => {
                self.check_expr(&bin.lhs, symptoms, locals, imports);
                self.check_expr(&bin.rhs, symptoms, locals, imports);
            }
            Expr::When(w) => {
                let subject_ty = w
                    .subject
                    .as_ref()
                    .map(|s| s.infer_ty(&self.infer_env(locals)));
                if let Some(subject) = &w.subject {
                    self.check_expr(subject, symptoms, locals, imports);
                }
                for arm in &w.arms {
                    match arm {
                        WhenArm::Cond { condition, body } => {
                            self.check_expr(condition, symptoms, locals, imports);
                            self.check_expr(body, symptoms, locals, imports);
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
                                    self.check_expr(e, symptoms, locals, imports);
                                });
                            } else {
                                symptoms.push(
                                    TypeSymptom::UnknownTag {
                                        name: "invalid is-pattern".to_string(),
                                    }
                                    .into_diagnostic(pattern.1),
                                );
                            }
                            self.check_expr(body, symptoms, locals, imports);
                        }
                        WhenArm::Else(body) => {
                            self.check_expr(body, symptoms, locals, imports);
                        }
                    }
                }
            }
            Expr::If(if_expr) => match &if_expr.condition {
                IfCondition::Bool(cond) => {
                    self.check_expr(cond, symptoms, locals, imports);
                    for e in &if_expr.body {
                        self.check_expr(e, symptoms, locals, imports);
                    }
                }
                IfCondition::Pattern { subject, pattern } => {
                    self.check_expr(subject, symptoms, locals, imports);
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
                            self.check_expr(e, symptoms, locals, imports);
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
                        self.check_expr(e, symptoms, &if_locals, imports);
                    }
                }
            },
            Expr::Loop(loop_expr) => match loop_expr {
                Loop::While(w) => {
                    self.check_expr(&w.cond, symptoms, locals, imports);
                    for e in &w.exprs {
                        self.check_expr(e, symptoms, locals, imports);
                    }
                }
                Loop::ForIn(f) => {
                    self.check_expr(&f.iter, symptoms, locals, imports);
                    for e in &f.exprs {
                        self.check_expr(e, symptoms, locals, imports);
                    }
                }
            },
            Expr::TupleLit(elems) => {
                for e in elems {
                    self.check_expr(e, symptoms, locals, imports);
                }
            }
            Expr::TupleAlloc { init, .. } => self.check_expr(init, symptoms, locals, imports),
            Expr::TupleGet { base, .. } => self.check_expr(base, symptoms, locals, imports),
            Expr::TupleSet { base, value, .. } => {
                self.check_expr(base, symptoms, locals, imports);
                self.check_expr(value, symptoms, locals, imports);
            }
            Expr::BufGet { buf, index, .. } => {
                self.check_expr(buf, symptoms, locals, imports);
                self.check_expr(index, symptoms, locals, imports);
            }
            Expr::BufSet {
                buf, index, value, ..
            } => {
                self.check_expr(buf, symptoms, locals, imports);
                self.check_expr(index, symptoms, locals, imports);
                self.check_expr(value, symptoms, locals, imports);
            }
            Expr::Cast { expr, .. } => self.check_expr(expr, symptoms, locals, imports),
            Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => {
                self.check_expr(e, symptoms, locals, imports);
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
                    self.check_expr(arg, symptoms, locals, imports);
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

    /// Verify the bind's body type is compatible with its declared return type.
    ///
    /// Today this only covers the tuple-IS-record case for top-level binds with
    /// a record-shaped return tag (`Range(x): (start, end)`). Callers rely on
    /// this to lock in that a positional tuple value satisfies a same-arity
    /// record return without an explicit record-literal in the body.
    fn check_body_matches_return(
        &self,
        bind: &Bind,
        symptoms: &mut Vec<diagnostic::Diagnostic>,
        locals: &dyn LocalTypes,
    ) {
        use diagnostic::DiagnosticLike;
        use diagnostic::type_::TypeSymptom;

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

    /// Check each argument of a function call against the called function's
    /// parameter types, sharing a single type-variable substitution so that
    /// `Range.new(1, "hi")` rejects (`x` cannot be both `Int` and `Str`) while
    /// `Range.new(12, 1200)` accepts.
    ///
    /// Silently no-ops when the called function isn't in the env (e.g.,
    /// imported from a not-yet-resolved module) — the unknown-binding check
    /// already covered that case.
    fn check_call_args(
        &self,
        mangled: &Intern<String>,
        call: &ast::FnCall,
        args: &[Spanned<Expr>],
        symptoms: &mut Vec<diagnostic::Diagnostic>,
        locals: &dyn LocalTypes,
    ) {
        use diagnostic::DiagnosticLike;
        use diagnostic::type_::TypeSymptom;

        let Some(info) = self.fn_params.get(mangled) else {
            return;
        };
        // For methods, the first parameter is the implicit `self`. Calls to
        // `Range.new(...)` don't pass it explicitly, so skip it.
        let mut params = info.params.iter();
        let mut bindings: HashMap<Intern<String>, Ty> = info.typevars.clone();
        // Mismatched arity is reported elsewhere; we just stop unifying.
        for arg in args {
            let Some((_, param_ty)) = params.next() else {
                break;
            };
            let arg_ty = arg.infer_ty(&self.infer_env(locals));
            if !ty_unifies_with(&arg_ty, param_ty, &mut bindings) {
                symptoms.push(TypeSymptom::Mismatch.into_diagnostic(arg.1));
            }
        }
        // Suppress dead_code on `call` for now — the parameter is here so we
        // can later attach richer diagnostics (named-argument hints, etc.).
        let _ = call;
    }
}

/// One-way structural unification check between an actual type and an expected
/// type, with type-variable bindings collected in `bindings`.
///
/// Tuple-IS-record: a `Ty::Tuple` matches a `Ty::Record` of the same arity by
/// positional field comparison (declaration order). This is the first step
/// toward fully unifying the two variants per the design intent that "tuple
/// is a record in Gin".
///
/// Type variables are represented by `Ty::Opaque(name)` on the *expected*
/// side. The first concrete type seen for a given name is bound; later
/// occurrences must match the bound type or unification fails. This is what
/// makes `Range.new(1, "hi")` reject (the second arg's `Str` cannot satisfy
/// `Opaque(x)` after the first arg already bound `x = Int`), while
/// `Range.new(12, 1200)` succeeds (`x = Int` for both).
pub(crate) fn ty_unifies_with(
    actual: &Ty,
    expected: &Ty,
    bindings: &mut HashMap<Intern<String>, Ty>,
) -> bool {
    if tys_equivalent(actual, expected) {
        return true;
    }
    match (actual, expected) {
        // Type-variable on the expected side: bind on first sight, then enforce
        // identity on subsequent occurrences. An identity binding (`x ->
        // Opaque(x)`) is treated as unbound — methods are typechecked with
        // identity bindings seeded so type-vars are visible, and call sites
        // need to overwrite them with concrete arg types.
        (_, Ty::Opaque(name)) => {
            let is_unbound = match bindings.get(name) {
                None => true,
                Some(Ty::Opaque(prev)) if prev == name => true,
                _ => false,
            };
            if is_unbound {
                // Strip the literal `value` field so later occurrences compare
                // by structure, not constant value (otherwise `Range.new(12, 1200)`
                // would reject because Int{value:12} != Int{value:1200}).
                bindings.insert(*name, strip_literal(actual));
                return true;
            }
            bindings
                .get(name)
                .map(|prev| tys_equivalent(prev, actual))
                .unwrap_or(false)
        }
        // Symmetric: opaque on the actual side (e.g. self-typed param) matches
        // anything for now (we don't yet do full bidirectional inference).
        (Ty::Opaque(_), _) => true,
        // Tuple-IS-record: positional match against declaration order.
        (Ty::Tuple(elems), Ty::Record { fields, .. }) => {
            if elems.len() != fields.len() {
                return false;
            }
            elems
                .iter()
                .zip(fields.iter())
                .all(|(e, (_, f))| ty_unifies_with(e, f, bindings))
        }
        // Symmetric record-vs-tuple, in case the actual side ends up that way.
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
        // Both sides are concrete ints/floats: require structural equality.
        // (Width and signedness are intentionally strict; users can `as` cast.)
        _ => false,
    }
}

/// Structural type equivalence ignoring literal `value` fields on Int/Float.
///
/// Constant folding stores literal values on `Ty::Int { value: Some(12) }` and
/// `Ty::Float`; for unification we only care about the carrier type.
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

/// Strip literal `value` constants from `Int`/`Float` types so a binding
/// captured from one literal call argument doesn't reject the next one.
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

fn collect_referenced_names(expr: &Expr, out: &mut HashSet<Intern<String>>) {
    match expr {
        Expr::FnCall(call) => {
            // The root identifier of any call/path access counts as a use of
            // that name. Field accesses (`r.start`) and method calls
            // (`r.method(args)`) both flow through `FnCall` with a non-empty
            // `segments` list, so we always insert the root regardless.
            out.insert(call.path.root);
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
