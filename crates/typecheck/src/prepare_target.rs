//! Default trait materialization and entry `target` merge from flask triple.

use std::collections::HashMap;

use diagnostic::Diagnostic;
use flask::CompileTarget;
use internment::Intern;

use crate::analysis::eval_compile_time_expr_public;
use crate::analysis::when_declare_is_exhaustive;
use crate::ty::Ty;
use ast::declare::{Declare, DeclareValue};
use ast::expr::{Expr, Literal, Typed};
use ast::span::SpanId;
use ast::ty_state::TyState;
use ast::type_decl::TypeNameExt;
use ast::{Bind, BindValue, FileAst, TypeExpr, WhenArm};
use ast::{ConstValue, HashFloat};

/// Fill unassigned binds whose type declares `has Default(default: …)`.
pub fn materialize_default_binds(ast: &mut FileAst) -> Vec<Diagnostic> {
    let names: Vec<_> = ast.defs.keys().copied().collect();
    let mut const_binds: HashMap<Intern<String>, Option<ConstValue>> =
        names.iter().map(|n| (*n, None)).collect();
    let mut diags = Vec::new();
    let mut updates: Vec<(Intern<String>, ConstValue, Expr)> = Vec::new();

    for name in names {
        let Some(bind) = ast.defs.get(&name) else {
            continue;
        };
        if !matches!(bind.value, BindValue::Unassigned) {
            continue;
        }
        let Some(type_name) = bind_return_type_name(bind) else {
            continue;
        };
        let Some(decl) = ast.tags.get(&type_name) else {
            continue;
        };
        let Some(default_expr) = default_expr_from_decl(decl) else {
            continue;
        };
        let Some(cv) = eval_compile_time_expr_public(&default_expr.value, &const_binds, ast) else {
            let name_span = bind.name_span;
            diags.push(
                Diagnostic::new(
                    "compile-time-custom",
                    format!("could not evaluate Default for `{}`", name.as_str()),
                )
                .at_span(ast.span_table.get(name_span)),
            );
            continue;
        };
        const_binds.insert(name, Some(cv.clone()));
        updates.push((name, cv.clone(), default_expr_value_expr(&cv)));
    }

    for (name, cv, expr_value) in updates {
        if let Some(bind) = ast.defs.get_mut(&name) {
            bind.value = BindValue::Expr(Box::new(Typed {
                value: expr_value,
                ty: TyState::Infer,
                const_value: Some(cv),
                span_id: bind.name_span,
            }));
        }
    }

    propagate_target_fields_in_ast(ast);
    diags
}

/// Merge entry flask triple fields into the stdlib `target` bind (special case).
pub fn apply_entry_target_merge(ast: &mut FileAst, entry: &CompileTarget) -> Vec<Diagnostic> {
    let CompileTarget::Concrete(triple) = entry else {
        return Vec::new();
    };

    let target_name = Intern::from_ref("target");
    let Some(bind) = ast.defs.get_mut(&target_name) else {
        return Vec::new();
    };

    let BindValue::Expr(expr) = &mut bind.value else {
        return Vec::new();
    };

    match expr.const_value.as_mut() {
        Some(ConstValue::Record { fields }) => {
            for (key, value) in triple.field_overrides() {
                if let Some((_, slot)) = fields.iter_mut().find(|(n, _)| n.as_str() == key) {
                    *slot = ConstValue::String(value);
                } else {
                    fields.push((Intern::from_ref(key), ConstValue::String(value)));
                }
            }
        }
        Some(ConstValue::Tag { name, args, .. }) if name.as_str() == "Target" => {
            let mut field_names: Vec<_> = ast
                .tags
                .get(name)
                .and_then(|decl| decl.params.as_ref())
                .map(|params| params.keys().copied().collect())
                .unwrap_or_default();
            if field_names.is_empty() {
                field_names = vec![
                    Intern::from_ref("arch"),
                    Intern::from_ref("vendor"),
                    Intern::from_ref("os"),
                ];
            }
            for (key, value) in triple.field_overrides() {
                if let Some(idx) = field_names.iter().position(|field| field.as_str() == key)
                    && let Some(slot) = args.get_mut(idx)
                {
                    *slot = ConstValue::String(value);
                }
            }
        }
        _ => return Vec::new(),
    }

    propagate_target_fields_in_ast(ast);
    propagate_when_subjects_in_tags(ast);
    Vec::new()
}

fn propagate_when_subjects_in_tags(ast: &mut FileAst) {
    for decl in ast.tags.values_mut() {
        if let DeclareValue::When(w) = &mut decl.value
            && let Some(subject) = &mut w.subject
        {
            propagate_record_fields_typed(subject);
        }
    }
}

fn bind_return_type_name(bind: &Bind) -> Option<Intern<String>> {
    let sp = bind.return_tag.as_ref()?;
    match &sp.value {
        TypeExpr::Nominal(name, _) => Some(*name),
        _ => None,
    }
}

fn default_expr_from_decl(decl: &Declare) -> Option<Typed<Expr>> {
    provided_trait_field_expr(decl, Intern::from_ref("default"))
}

fn provided_trait_field_expr(decl: &Declare, field: Intern<String>) -> Option<Typed<Expr>> {
    decl.provided_traits.iter().find_map(|pt| {
        pt.fields
            .iter()
            .find(|(n, _)| *n == field)
            .map(|(_, e)| e.clone())
    })
}

type TypeStaticKey = (Intern<String>, Intern<String>);
type TypeStaticExpansions = HashMap<TypeStaticKey, ConstValue>;

fn build_type_static_expansions(
    ast: &FileAst,
    const_binds: &HashMap<Intern<String>, Option<ConstValue>>,
) -> TypeStaticExpansions {
    let mut out = TypeStaticExpansions::new();
    for (type_name, decl) in &ast.tags {
        if !type_name.as_str().is_capitalized_type_name() {
            continue;
        }
        for pt in &decl.provided_traits {
            for (field, expr) in &pt.fields {
                if let Some(cv) = eval_compile_time_expr_public(&expr.value, const_binds, ast) {
                    out.insert((*type_name, *field), cv);
                }
            }
        }
    }
    out
}

fn lookup_type_static_expansion(
    expansions: &TypeStaticExpansions,
    type_name: Intern<String>,
    field: Intern<String>,
) -> Option<ConstValue> {
    if !type_name.as_str().is_capitalized_type_name() {
        return None;
    }
    expansions.get(&(type_name, field)).cloned()
}

/// Evaluate `TypeName.field` when `field` is provided on the tag's `and has Trait(…)` clause.
pub(crate) fn eval_type_static_member(
    type_name: Intern<String>,
    field: Intern<String>,
    ast: &FileAst,
    const_binds: &HashMap<Intern<String>, Option<ConstValue>>,
) -> Option<ConstValue> {
    let expansions = build_type_static_expansions(ast, const_binds);
    lookup_type_static_expansion(&expansions, type_name, field)
}

/// Expand `Target.default`-style paths to concrete compile-time expressions.
pub fn materialize_type_static_access(ast: &mut FileAst) {
    let const_binds: HashMap<Intern<String>, Option<ConstValue>> =
        ast.defs.keys().map(|n| (*n, None)).collect();
    let expansions = build_type_static_expansions(ast, &const_binds);
    let bind_names: Vec<_> = ast.defs.keys().copied().collect();
    for name in bind_names {
        if let Some(bind) = ast.defs.get_mut(&name) {
            materialize_type_static_access_in_bind(bind, &expansions);
        }
    }
    for (expr, _) in &mut ast.exprs {
        materialize_type_static_access_expr(expr, &expansions);
    }
    let tag_names: Vec<_> = ast.tags.keys().copied().collect();
    for tag_name in tag_names {
        let Some(decl) = ast.tags.get_mut(&tag_name) else {
            continue;
        };
        if let DeclareValue::When(w) = &mut decl.value {
            if let Some(subject) = &mut w.subject {
                materialize_type_static_access_typed(subject, &expansions);
            }
            for arm in &mut w.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        materialize_type_static_access_typed(condition, &expansions);
                        materialize_type_static_access_typed(body, &expansions);
                    }
                    WhenArm::Is { body, .. } => {
                        materialize_type_static_access_typed(body, &expansions);
                    }
                    WhenArm::Else(body, _) => {
                        materialize_type_static_access_typed(body, &expansions);
                    }
                }
            }
        }
    }
}

fn materialize_type_static_access_in_bind(bind: &mut Bind, expansions: &TypeStaticExpansions) {
    match &mut bind.value {
        BindValue::Expr(e) => materialize_type_static_access_typed(e, expansions),
        BindValue::Body { exprs, ret } => {
            for e in exprs {
                materialize_type_static_access_typed(e, expansions);
            }
            if let Some(r) = &mut ret.value {
                materialize_type_static_access_typed(r, expansions);
            }
        }
        BindValue::Extern | BindValue::Unassigned => {}
    }
}

fn materialize_type_static_access_typed(expr: &mut Typed<Expr>, expansions: &TypeStaticExpansions) {
    if let Expr::FnCall(call) = &expr.value
        && call.args.is_none()
        && call.path.value.segments.len() == 1
    {
        let type_name = call.path.value.root;
        let field = call.path.value.segments[0];
        if let Some(cv) = lookup_type_static_expansion(expansions, type_name, field) {
            *expr = Typed {
                value: default_expr_value_expr(&cv),
                ty: expr.ty.clone(),
                const_value: Some(cv),
                span_id: expr.span_id,
            };
            propagate_record_fields_typed(expr);
            return;
        }
    }
    materialize_type_static_access_expr(&mut expr.value, expansions);
}

fn materialize_type_static_access_expr(expr: &mut Expr, expansions: &TypeStaticExpansions) {
    if let Expr::FnCall(call) = expr
        && call.args.is_none()
        && call.path.value.segments.len() == 1
    {
        let type_name = call.path.value.root;
        let field = call.path.value.segments[0];
        if let Some(cv) = lookup_type_static_expansion(expansions, type_name, field) {
            *expr = default_expr_value_expr(&cv);
            return;
        }
    }
    match expr {
        Expr::FnCall(c) => {
            if let Some(args) = &mut c.args {
                for a in args {
                    materialize_type_static_access_typed(a, expansions);
                }
            }
        }
        Expr::Binary(b) => {
            materialize_type_static_access_typed(&mut b.lhs, expansions);
            materialize_type_static_access_typed(&mut b.rhs, expansions);
        }
        Expr::Bind(b) => {
            if let BindValue::Expr(inner) = &mut b.value {
                materialize_type_static_access_typed(inner, expansions);
            }
        }
        Expr::When(w) => {
            if let Some(s) = &mut w.subject {
                materialize_type_static_access_typed(s, expansions);
            }
            for arm in &mut w.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        materialize_type_static_access_typed(condition, expansions);
                        materialize_type_static_access_typed(body, expansions);
                    }
                    WhenArm::Is { body, .. } => {
                        materialize_type_static_access_typed(body, expansions);
                    }
                    WhenArm::Else(body, _) => {
                        materialize_type_static_access_typed(body, expansions);
                    }
                }
            }
        }
        Expr::If(i) => {
            materialize_type_static_access_typed(&mut i.subject, expansions);
            for e in &mut i.body {
                materialize_type_static_access_typed(e, expansions);
            }
            if let Some(r) = &mut i.ret.value {
                materialize_type_static_access_typed(r, expansions);
            }
        }
        Expr::Destructure { value, .. } => {
            materialize_type_static_access_typed(value, expansions);
        }
        Expr::RecordSet { base, value, .. } => {
            materialize_type_static_access_typed(base, expansions);
            materialize_type_static_access_typed(value, expansions);
        }
        Expr::RecordGet { base, .. } => materialize_type_static_access_typed(base, expansions),
        Expr::TagCall(tc) => {
            for a in &mut tc.args {
                materialize_type_static_access_typed(a, expansions);
            }
        }
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for e in elems {
                materialize_type_static_access_typed(e, expansions);
            }
        }
        _ => {}
    }
}

fn default_expr_value_expr(cv: &ConstValue) -> Expr {
    match cv {
        ConstValue::String(s) => Expr::Lit(Literal::String(s.clone())),
        ConstValue::Int(n) => {
            let n = u128::try_from(*n).unwrap_or(0);
            Expr::Lit(Literal::Int(n))
        }
        ConstValue::Float(HashFloat(f)) => Expr::Lit(Literal::Float(HashFloat(*f))),
        ConstValue::Tag { name, args, .. } if args.is_empty() => Expr::AnonymousTag(*name),
        ConstValue::Tag {
            name,
            qual_path,
            args,
        } => Expr::TagCall(ast::TagCall {
            name: *name,
            qual_path: qual_path.as_ref().map(|s| {
                let parts: Vec<_> = s.split('.').map(Intern::from_ref).collect();
                let root = parts[0];
                let segments = parts.get(1..).unwrap_or(&[]).to_vec();
                ast::Spanned::new(ast::ModPath::new(root, segments), SpanId::INVALID)
            }),
            args: args
                .iter()
                .map(|a| Typed::infer(default_expr_value_expr(a), SpanId::INVALID))
                .collect(),
        }),
        ConstValue::Record { fields } => {
            // Reconstruct a record literal with named fields.
            Expr::RecordLit(
                fields
                    .iter()
                    .map(|(name, v)| {
                        let value = default_expr_value_expr(v);
                        let typed = Typed::infer(value, SpanId::INVALID);
                        (*name, typed)
                    })
                    .collect(),
            )
        }
        ConstValue::List(items) => Expr::List(
            items
                .iter()
                .map(|i| Typed::infer(default_expr_value_expr(i), SpanId::INVALID))
                .collect(),
        ),
    }
}

fn propagate_target_fields_in_ast(ast: &mut FileAst) {
    for bind in ast.defs.values_mut() {
        if let BindValue::Expr(e) = &mut bind.value {
            propagate_record_fields_typed(e);
        } else if let BindValue::Body { exprs, ret } = &mut bind.value {
            for e in exprs {
                propagate_record_fields_typed(e);
            }
            if let Some(r) = &mut ret.value {
                propagate_record_fields_typed(r);
            }
        }
    }
}

/// Compile-time bind values from a prepared package AST (for cross-file `when` subjects).
pub fn const_binds_from_prepared_ast(ast: &FileAst) -> HashMap<Intern<String>, Option<ConstValue>> {
    let mut const_binds: HashMap<Intern<String>, Option<ConstValue>> =
        ast.defs.keys().map(|n| (*n, None)).collect();
    for _ in 0..ast.defs.len().max(1) {
        let names: Vec<_> = ast.defs.keys().copied().collect();
        for name in names {
            if const_binds.get(&name).and_then(|v| v.as_ref()).is_some() {
                continue;
            }
            let Some(bind) = ast.defs.get(&name) else {
                continue;
            };
            let cv = match &bind.value {
                BindValue::Expr(e) => e
                    .const_value
                    .clone()
                    .or_else(|| eval_compile_time_expr_public(&e.value, &const_binds, ast)),
                _ => None,
            };
            if let Some(cv) = cv {
                const_binds.insert(name, Some(cv));
            }
        }
    }
    const_binds
}

/// Subject type for `when target.arch is` — the `Architecture` literal union when present.
pub fn infer_when_declare_subject_ty(
    subject: &Option<Box<Typed<Expr>>>,
    tags: &ast::TagMap,
) -> Option<Ty> {
    let subject = subject.as_ref()?;
    let is_target_arch = match &subject.value {
        Expr::RecordGet { field, .. } => field.as_str() == "arch",
        Expr::FnCall(call) if call.args.is_none() && call.path.root.as_str() == "target" => call
            .path
            .segments
            .first()
            .is_some_and(|s| s.as_str() == "arch"),
        _ => false,
    };
    if !is_target_arch {
        return None;
    }
    literal_union_ty_from_tag(Intern::from_ref("Architecture"), tags)
}

fn literal_union_ty_from_tag(name: Intern<String>, tags: &ast::TagMap) -> Option<Ty> {
    let decl = tags.get(&name)?;
    let DeclareValue::Union { variants } = &decl.value else {
        return None;
    };
    let mut lit_values = Vec::new();
    let mut lit_base = None;
    for variant in variants {
        let shape = variant.shape();
        if let TypeExpr::Literal(Literal::String(s), _) = &shape.value {
            lit_values.push(ConstValue::String(s.clone()));
            if lit_base.is_none() {
                lit_base = Some(Ty::Opaque(Intern::<String>::from_ref("Str")));
            }
        } else if let TypeExpr::Literal(lit, _) = &shape.value {
            if let Some(cv) = const_value_from_literal(lit) {
                lit_values.push(cv);
            }
        } else {
            return None;
        }
    }
    if lit_values.is_empty() {
        return None;
    }
    let _ = lit_base;
    Some(Ty::union_of_literals(name, lit_values))
}

fn const_value_from_literal(lit: &Literal) -> Option<ConstValue> {
    match lit {
        Literal::String(s) => Some(ConstValue::String(s.clone())),
        Literal::Int(n) => Some(ConstValue::Int(*n as i128)),
        Literal::Float(HashFloat(f)) => Some(ConstValue::Float(HashFloat(*f))),
        Literal::Number(n) => Some(ConstValue::Int(*n as i128)),
    }
}

/// `MissingElseArm` / `UnreachableElseArm` for type-level `when` before compile-time simplification.
pub fn validate_when_declare_exhaustiveness(
    ast: &FileAst,
    tag_source: &ast::TagMap,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for decl in ast.tags.values() {
        let DeclareValue::When(w) = &decl.value else {
            continue;
        };
        let subject_ty = infer_when_declare_subject_ty(&w.subject, tag_source);
        let has_else = w.arms.iter().any(|a| matches!(a, WhenArm::Else(..)));
        let exhaustive = when_declare_is_exhaustive(subject_ty.as_ref(), &w.arms);
        if !has_else && !exhaustive {
            let span = w
                .subject
                .as_ref()
                .map(|s| ast.span_table.get(s.span_id))
                .unwrap_or_else(|| ast.span_table.get(w.body_span.into_inner()));
            diags.push(
                Diagnostic::new("type-missing-else-arm", "`when` should have an `else` arm")
                    .at_span(span),
            );
        } else if has_else && exhaustive {
            let Some(else_span) = w.arms.iter().find_map(|a| match a {
                WhenArm::Else(_, span) => Some(*span),
                _ => None,
            }) else {
                continue;
            };
            diags.push(
                Diagnostic::new("type-unreachable-else-arm", "`else` arm is unreachable")
                    .at_span(ast.span_table.get(else_span.into_inner())),
            );
        }
    }
    diags
}

/// Fold `when` declare subjects (e.g. `target.arch`) using defs from the whole prepared package.
pub fn materialize_when_declare_subjects_from_package(ast: &mut FileAst, package: &FileAst) {
    let const_binds = const_binds_from_prepared_ast(package);
    for decl in ast.tags.values_mut() {
        if let DeclareValue::When(w) = &mut decl.value
            && let Some(subject) = &mut w.subject
        {
            if subject.const_value.is_none()
                && let Some(cv) =
                    eval_compile_time_expr_public(&subject.value, &const_binds, package)
            {
                subject.const_value = Some(cv);
            }
            propagate_record_fields_typed(subject);
        }
    }
}

fn propagate_record_fields_typed(expr: &mut Typed<Expr>) {
    if let Expr::RecordGet { base, field } = &expr.value {
        let record = base.const_value.as_ref().or_else(|| {
            if let Expr::Bind(b) = &base.value
                && b.params.is_none()
                && let BindValue::Expr(inner) = &b.value
            {
                inner.const_value.as_ref()
            } else {
                None
            }
        });
        if let Some(ConstValue::Record { fields }) = record
            && let Some((_, v)) = fields.iter().find(|(n, _)| n == field)
        {
            expr.const_value = Some(v.clone());
        }
    }
    match &mut expr.value {
        Expr::FnCall(c) => {
            if let Some(args) = &mut c.args {
                for a in args {
                    propagate_record_fields_typed(a);
                }
            }
        }
        Expr::Binary(b) => {
            propagate_record_fields_typed(&mut b.lhs);
            propagate_record_fields_typed(&mut b.rhs);
        }
        Expr::Bind(b) => {
            if let BindValue::Expr(inner) = &mut b.value {
                propagate_record_fields_typed(inner);
            }
        }
        Expr::When(w) => {
            if let Some(s) = &mut w.subject {
                propagate_record_fields_typed(s);
            }
            for arm in &mut w.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        propagate_record_fields_typed(condition);
                        propagate_record_fields_typed(body);
                    }
                    WhenArm::Is { body, .. } => propagate_record_fields_typed(body),
                    WhenArm::Else(body, _) => propagate_record_fields_typed(body),
                }
            }
        }
        Expr::If(i) => {
            propagate_record_fields_typed(&mut i.subject);
            for e in &mut i.body {
                propagate_record_fields_typed(e);
            }
            if let Some(r) = &mut i.ret.value {
                propagate_record_fields_typed(r);
            }
        }
        Expr::Destructure { value, .. } => {
            propagate_record_fields_typed(value);
        }
        Expr::RecordSet { base, value, .. } => {
            propagate_record_fields_typed(base);
            propagate_record_fields_typed(value);
        }
        Expr::RecordGet { base, .. } => propagate_record_fields_typed(base),
        Expr::TagCall(tc) => {
            for a in &mut tc.args {
                propagate_record_fields_typed(a);
            }
        }
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for e in elems {
                propagate_record_fields_typed(e);
            }
        }
        _ => {}
    }
}
