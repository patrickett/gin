use std::collections::HashSet;

use ast::{Expr, FileAst, Literal, ParameterKind, Spanned};
use internment::Intern;

use crate::flow::ConstValue;
use crate::ty::Ty;
use crate::TyEnv;

/// Import information for the current file.
pub(crate) struct ImportSet {
    pub all: HashSet<Intern<String>>,
    pub bundle_members: HashSet<Intern<String>>,
    pub module_prefixes: HashSet<Intern<String>>,
    pub alias_spans: HashSet<ast::SpanId>,
}

/// Collect import names and bundle member names from the AST.
pub(crate) fn collect_import_names(ast: &FileAst) -> ImportSet {
    let mut all = HashSet::new();
    let mut bundle_members = HashSet::new();
    let mut module_prefixes = HashSet::new();
    let alias_spans: HashSet<ast::SpanId> = ast.symbol_alias_spans.iter().copied().collect();
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
pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
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

pub(crate) fn suggest_typo_for_identifier(
    ty_env: &TyEnv,
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
    for k in ty_env.fn_return_types.keys() {
        let s = k.to_string();
        if seen.insert(s.clone()) {
            candidates.push(s);
        }
    }
    for k in ty_env.tag_types.keys() {
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

/// Check whether `segment` names a field of the given type.
pub(crate) fn is_field_of_type(ty: &Ty, segment: &Intern<String>) -> bool {
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

pub(crate) fn fmt_call_without_parens(call: &ast::FnCall) -> String {
    if call.path.segments.is_empty() {
        call.path.root.as_str().to_string()
    } else {
        let segs: Vec<&str> = call.path.segments.iter().map(|s| s.as_str()).collect();
        format!("{}.{}", call.path.root.as_str(), segs.join("."))
    }
}

pub(crate) fn check_type_pattern_default_exprs(
    surface: &Expr,
    check: &mut impl FnMut(&Spanned<Expr>),
) {
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

pub(crate) fn check_type_surface_defaults(e: &Expr, check: &mut impl FnMut(&Spanned<Expr>)) {
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

/// Produce a human-readable type name for use in diagnostics.
pub(crate) fn type_name_for_display(ty: &Ty) -> String {
    match ty {
        Ty::Int { .. } => "Int".into(),
        Ty::Float { .. } => "Float".into(),
        Ty::Bool => "Bool".into(),
        Ty::Unit => "Unit".into(),
        Ty::Record { name, .. } | Ty::Union { name, .. } | Ty::ConstUnion { name, .. } => {
            name.as_str().to_string()
        }
        Ty::Opaque(name) => name.as_str().to_string(),
        Ty::Array { .. } => "Array".into(),
        Ty::Ptr { .. } => "Ptr".into(),
        Ty::Ref { .. } => "Ref".into(),
        Ty::Tuple(fields) => {
            let inner: Vec<String> = fields.iter().map(type_name_for_display).collect();
            format!("({})", inner.join(", "))
        }
    }
}

/// Check whether an expression is a literal whose value belongs to a ConstUnion's value set.
pub(crate) fn literal_matches_const_union(expr: &Expr, values: &[ConstValue]) -> bool {
    match expr {
        Expr::Lit(Literal::String(s)) => values
            .iter()
            .any(|v| matches!(v, ConstValue::String(vs) if vs == s)),
        Expr::Lit(Literal::Int(n)) => values
            .iter()
            .any(|v| matches!(v, ConstValue::Int(vn) if *vn == *n as i128)),
        Expr::Lit(Literal::Float(f)) => values
            .iter()
            .any(|v| matches!(v, ConstValue::Float(vf) if vf.to_bits() == f.to_bits())),
        Expr::Lit(Literal::Number(n)) => values
            .iter()
            .any(|v| matches!(v, ConstValue::Int(vn) if *vn == *n as i128)),
        _ => false,
    }
}
