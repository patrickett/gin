//! Unified surface (patterns, type-shaped positions, values):
//!
//! - **Values** — [`Expr`] in `crates/ast/src/expr/mod.rs`.
//! - **For binders** — [`Expr`] on [`crate::expr::ForInLoop`] (`for_loop_pattern_names`).
//! - **`if` / `when` `is` patterns** — [`Pattern`].
//! - **Bind return / receiver** — structural [`TypeExpr`] type surfaces.
//!
//! The parser builds a structural type surface first, then converts the pattern
//! boundary into this dedicated representation.
//!
//! Ref: https://matklad.github.io/2025/08/09/zigs-lovely-syntax.html#Everything-Is-an-Expression

use internment::Intern;
use std::collections::HashMap;

use crate::{GroupPath, InRangeBounds, ModPath, TypeExpr};
use crate::expr::{Expr, FnCall, Literal, Typed, WhenArm, WhenExpr};
use crate::parameter::ParameterKind;
use crate::normal_expr::NormalExpr;
use crate::source::SourceExt;
use crate::span::{SpanId, SpanTable, Spanned};
use crate::ty::{Ty, VariantMap};
use crate::{BindValue, DeclareValue, FileAst};

/// A pattern-bearing syntax node kept distinct from ordinary type surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Pattern {
    Nominal(Intern<String>, SpanId),
    Qualified(crate::Spanned<ModPath>),
    Generic {
        name: Intern<String>,
        params: Vec<(Intern<String>, ParameterKind)>,
        param_spans: Vec<(Intern<String>, SpanId)>,
        span: SpanId,
    },
    Literal(Literal, SpanId),
    Pointer(Box<crate::Spanned<Pattern>>),
    Ref {
        inner: Box<crate::Spanned<Pattern>>,
        mutable: bool,
        group: Option<GroupPath>,
    },
    Unit,
    ListEmpty,
    ListCons {
        head: Box<crate::Spanned<Pattern>>,
        tail: Box<crate::Spanned<Pattern>>,
    },
    Tuple(Vec<crate::Spanned<Pattern>>),
    InRange { bounds: InRangeBounds, span: SpanId },
}

impl Pattern {
    pub fn variant_shape_name_span(&self, variant_name: &str) -> Option<SpanId> {
        match self {
            Self::Nominal(name, span) | Self::Generic { name, span, .. }
                if name.as_str() == variant_name => Some(*span),
            Self::Literal(Literal::String(value), span) if value == variant_name => Some(*span),
            _ => None,
        }
    }

    pub fn from_expr(expr: Expr) -> Self {
        match expr {
            Expr::AnonymousTag(name) => Self::Nominal(name, SpanId::INVALID),
            Expr::FnCall(call) if call.args.as_ref().is_none_or(Vec::is_empty) => {
                if call.path.value.segments.is_empty() {
                    Self::Nominal(call.path.value.root, call.path.span_id)
                } else {
                    Self::Qualified(call.path)
                }
            }
            Expr::TagCall(call) if call.args.is_empty() => {
                if let Some(path) = call.qual_path {
                    Self::Qualified(path)
                } else {
                    Self::Nominal(call.name, SpanId::INVALID)
                }
            }
            Expr::TagCall(call) => {
                let params = call
                    .args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| {
                        let name = Intern::new(format!("_{index}"));
                        (name, ParameterKind::Default(Box::new(arg.clone())))
                    })
                    .collect::<Vec<_>>();
                let param_spans = params
                    .iter()
                    .map(|(name, _)| (*name, SpanId::INVALID))
                    .collect();
                Self::Generic {
                    name: call.name,
                    params,
                    param_spans,
                    span: SpanId::INVALID,
                }
            }
            Expr::Lit(lit) => Self::Literal(lit, SpanId::INVALID),
            Expr::Ref {
                inner,
                mutable,
                group,
            } => Self::Ref {
                inner: Box::new(Spanned {
                    value: Self::from_expr(inner.value),
                    span_id: inner.span_id,
                }),
                mutable,
                group,
            },
            Expr::TakePtr(inner) => Self::Pointer(Box::new(Spanned {
                value: Self::from_expr(inner.value),
                span_id: inner.span_id,
            })),
            Expr::TupleLit(values) => Self::Tuple(
                values
                    .into_iter()
                    .map(|value| Spanned {
                        value: Self::from_expr(value.value),
                        span_id: value.span_id,
                    })
                    .collect(),
            ),
            Expr::List(values) => values
                .into_iter()
                .rev()
                .fold(Self::ListEmpty, |tail, value| Self::ListCons {
                    head: Box::new(Spanned {
                        value: Self::from_expr(value.value),
                        span_id: value.span_id,
                    }),
                    tail: Box::new(Spanned {
                        value: tail,
                        span_id: value.span_id,
                    }),
                }),
            _ => Self::Nominal(Intern::from_ref("_"), SpanId::INVALID),
        }
    }

    pub fn is_catch_all_pattern(&self) -> bool {
        match self {
            Self::Nominal(name, _) if name.as_str() == "_" => true,
            Self::ListCons { tail, .. } => tail.value.is_catch_all_pattern(),
            Self::Generic { params, .. } => {
                params.len() == 1
                    && params[0].0.as_str() == "_"
                    && matches!(params[0].1, ParameterKind::Generic)
            }
            _ => false,
        }
    }

    pub fn pattern_binding_types(
        &self,
        subject_ty: Option<&Ty>,
        variant_map: &VariantMap,
        tag_types: &HashMap<Intern<String>, Ty>,
    ) -> HashMap<Intern<String>, Ty> {
        let mut out = HashMap::new();
        match self {
            Pattern::Generic {
                params,
                param_spans,
                ..
            } => {
                let variant_name = Intern::<String>::from_ref(self.surface_mangle_name());
                if let Some(payload_fields) = crate::type_expr::payload_fields_for_variant(
                    variant_name,
                    subject_ty,
                    variant_map,
                ) {
                    for (slot, (param_name, _span)) in param_spans.iter().enumerate() {
                        let ty = if let Some((_, ty)) = payload_fields.get(slot) {
                            ty.clone()
                        } else if let Some((_, kind)) =
                            params.iter().find(|(name, _)| name == param_name)
                        {
                            match kind {
                                ParameterKind::Tagged(sp) => {
                                    crate::type_expr::resolve_expr_type(&sp.value, tag_types)
                                }
                                ParameterKind::ValueParam { ty }
                                | ParameterKind::Inferred { ty } => {
                                    crate::type_expr::resolve_expr_type(&ty.value, tag_types)
                                }
                                ParameterKind::Generic => Ty::Opaque(*param_name),
                                ParameterKind::Default(_) => continue,
                            }
                        } else {
                            continue;
                        };
                        if !param_name.is_pattern_wildcard() {
                            out.insert(*param_name, ty);
                        }
                    }
                } else {
                    for (param_name, kind) in params {
                        if param_name.is_pattern_wildcard() {
                            continue;
                        }
                        if let ParameterKind::Tagged(sp) = kind {
                            out.insert(
                                *param_name,
                                crate::type_expr::resolve_expr_type(&sp.value, tag_types),
                            );
                        } else {
                            out.insert(*param_name, Ty::Opaque(*param_name));
                        }
                    }
                }
            }
            Pattern::Nominal(name, _)
                if name
                    .as_str()
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase())
                    && !name.is_pattern_wildcard() =>
            {
                out.insert(
                    *name,
                    subject_ty
                        .cloned()
                        .unwrap_or(Ty::Opaque(*name)),
                );
            }
            Pattern::ListCons { head, tail } => {
                if let Some(elem_ty) = subject_ty.and_then(|ty| ty.list_elem_ty(tag_types)) {
                    for (name, ty) in head.value.pattern_binding_types(
                        Some(&elem_ty),
                        variant_map,
                        tag_types,
                    ) {
                        out.insert(name, ty);
                    }
                    let tail_ty = match subject_ty {
                        Some(Ty::Array { elem, .. }) => Ty::Array {
                            elem: elem.clone(),
                            size: NormalExpr::from(0),
                        },
                        Some(ty) => ty.clone(),
                        None => Ty::Opaque(Intern::new("list_tail".to_string())),
                    };
                    for (name, ty) in tail.value.pattern_binding_types(
                        Some(&tail_ty),
                        variant_map,
                        tag_types,
                    ) {
                        out.insert(name, ty);
                    }
                }
            }
            Pattern::Tuple(elems) => {
                if let Some(Ty::Tuple(subject_elems)) = subject_ty {
                    for (pat, subj) in elems.iter().zip(subject_elems.iter()) {
                        for (name, ty) in pat.value.pattern_binding_types(
                            Some(subj),
                            variant_map,
                            tag_types,
                        ) {
                            out.insert(name, ty);
                        }
                    }
                }
            }
            _ => {}
        }
        out
    }

    pub fn pattern_param_type_at_slot(
        &self,
        slot: usize,
        subject_ty: Option<&Ty>,
        variant_map: &VariantMap,
        tag_types: &HashMap<Intern<String>, Ty>,
    ) -> Option<Ty> {
        match self {
            Pattern::Generic {
                params,
                param_spans,
                ..
            } => {
                let (param_name, _) = param_spans.get(slot)?;
                let variant_name = Intern::<String>::from_ref(self.surface_mangle_name());
                if let Some(payload_fields) = crate::type_expr::payload_fields_for_variant(
                    variant_name,
                    subject_ty,
                    variant_map,
                ) && let Some((_, ty)) = payload_fields.get(slot)
                {
                    return Some(ty.clone());
                }
                let kind = params.iter().find(|(name, _)| name == param_name).map(|(_, k)| k);
                match kind {
                    Some(ParameterKind::Tagged(sp)) => Some(
                        crate::type_expr::resolve_expr_type(&sp.value, tag_types),
                    ),
                    Some(ParameterKind::Generic) if !param_name.is_pattern_wildcard() => {
                        Some(Ty::Opaque(*param_name))
                    }
                    _ => None,
                }
            }
            Pattern::ListCons { head, tail } => {
                let elem_ty = subject_ty.and_then(|ty| ty.list_elem_ty(tag_types))?;
                if slot == 0 {
                    head.value.pattern_param_type_at_slot(
                        0,
                        Some(&elem_ty),
                        variant_map,
                        tag_types,
                    )
                } else {
                    let tail_ty = match subject_ty {
                        Some(Ty::Array { elem, .. }) => Ty::Array {
                            elem: elem.clone(),
                            size: NormalExpr::from(0),
                        },
                        _ => tag_types
                            .get(&Intern::from_ref("List"))
                            .cloned()
                            .unwrap_or(Ty::Opaque(Intern::new("list_tail".to_string()))),
                    };
                    tail.value.pattern_param_type_at_slot(
                        slot.saturating_sub(1),
                        Some(&tail_ty),
                        variant_map,
                        tag_types,
                    )
                }
            }
            _ => None,
        }
    }

    pub fn surface_mangle_name(&self) -> &str {
        match self {
            Self::Nominal(name, _) => name.as_str(),
            Self::Generic { name, .. } => name.as_str(),
            Self::Qualified(path) => path
                .segments
                .last()
                .map(|s| s.as_str())
                .unwrap_or(path.root.as_str()),
            Self::Literal(lit, _) => match lit {
                Literal::String(s) => s.as_str(),
                Literal::Int(_) => "__literal_int",
                Literal::Float(_) => "__literal_float",
                Literal::Number(_) => "__literal_number",
            },
            Self::Pointer(inner) | Self::Ref { inner, .. } => {
                inner.value.surface_mangle_name()
            }
            Self::Unit => "()",
            Self::InRange { .. } => "__in_range",
            Self::ListEmpty => "[]",
            Self::ListCons { .. } => "[..]",
            Self::Tuple(_) => "(..)",
        }
    }

    pub fn nominal_name(&self) -> Option<&Intern<String>> {
        match self {
            Self::Nominal(name, _) => Some(name),
            _ => None,
        }
    }

    pub fn nominal_with_span(&self) -> Option<(&Intern<String>, SpanId)> {
        match self {
            Self::Nominal(name, span) => Some((name, *span)),
            _ => None,
        }
    }

    pub fn is_list_empty(&self) -> bool {
        matches!(self, Self::ListEmpty)
    }

    pub fn is_list_cons(&self) -> bool {
        matches!(self, Self::ListCons { .. })
    }

    pub fn is_list_tail_catch_all(&self) -> bool {
        match self {
            Self::Nominal(name, _) => name
                .as_str()
                .chars()
                .next()
                .is_some_and(|ch| ch == '_' || ch.is_ascii_lowercase()),
            _ => self.is_catch_all_pattern(),
        }
    }

    pub fn list_cons_shape(&self) -> Option<(usize, bool)> {
        fn is_tail_catch_all(expr: &Pattern) -> bool {
            match expr {
                Pattern::Nominal(name, _) => name
                    .as_str()
                    .chars()
                    .next()
                    .is_some_and(|ch| ch == '_' || ch.is_ascii_lowercase()),
                _ => expr.is_catch_all_pattern(),
            }
        }

        fn shape(expr: &Pattern) -> Option<(usize, bool)> {
            match expr {
                Pattern::ListEmpty => Some((0, false)),
                Pattern::ListCons { tail, .. } => {
                    let (depth, catch_all) = shape(&tail.value)?;
                    Some((depth + 1, catch_all))
                }
                _ => Some((0, is_tail_catch_all(expr))),
            }
        }

        shape(self)
    }

    pub(crate) fn variant_word_at(
        &self,
        pattern_span_id: SpanId,
        span_table: &SpanTable,
        byte_pos: usize,
        word: &str,
    ) -> bool {
        if !span_table.contains(pattern_span_id, byte_pos) {
            return false;
        }
        match self {
            Pattern::Generic {
                name,
                params,
                param_spans,
                span,
                ..
            } => {
                let head_span = span_table.get(*span);
                if head_span.start() <= byte_pos
                    && byte_pos < head_span.end()
                    && word == name.as_str()
                {
                    return true;
                }
                param_spans.iter().enumerate().any(|(slot, (pname, pspan))| {
                    let ps = span_table.get(*pspan);
                    if ps.start() <= byte_pos
                        && byte_pos < ps.end()
                        && (word == pname.as_str()
                            || (pname.is_pattern_wildcard() && word == "_"))
                    {
                        return params
                            .get(slot)
                            .and_then(|(_, kind)| match kind {
                                ParameterKind::Tagged(sp) => {
                                    Some(sp.value.denotes_variant_name(word))
                                }
                                _ => None,
                            })
                            .unwrap_or(false);
                    }
                    false
                })
            }
            Pattern::ListCons { head, tail } => {
                head.value
                    .variant_word_at(pattern_span_id, span_table, byte_pos, word)
                    || tail
                        .value
                        .variant_word_at(pattern_span_id, span_table, byte_pos, word)
            }
            Pattern::Tuple(elems) => elems.iter().any(|elem| {
                elem.value
                    .variant_word_at(pattern_span_id, span_table, byte_pos, word)
            }),
            _ => false,
        }
    }
}

impl From<TypeExpr> for Pattern {
    fn from(value: TypeExpr) -> Self {
        match value {
            TypeExpr::Nominal(name, span) => Self::Nominal(name, span),
            TypeExpr::Qualified(path) => Self::Qualified(path),
            TypeExpr::Generic {
                name,
                params,
                param_spans,
                span,
            } => Self::Generic {
                name,
                params,
                param_spans,
                span,
            },
            TypeExpr::Literal(lit, span) => Self::Literal(lit, span),
            TypeExpr::Pointer(inner) => Self::Pointer(Box::new(crate::Spanned {
                value: Self::from(inner.value),
                span_id: inner.span_id,
            })),
            TypeExpr::Ref {
                inner,
                mutable,
                group,
            } => Self::Ref {
                inner: Box::new(crate::Spanned {
                    value: Self::from(inner.value),
                    span_id: inner.span_id,
                }),
                mutable,
                group,
            },
            TypeExpr::Unit => Self::Unit,
            TypeExpr::InRange { bounds, span } => Self::InRange { bounds, span },
        }
    }
}

impl TypeExpr {
    /// Return the mangled surface name for this type expression.
    pub fn surface_mangle_name(&self) -> &str {
        match self {
            TypeExpr::Nominal(n, _) => n.as_str(),
            TypeExpr::Generic { name, .. } => name.as_str(),
            TypeExpr::Qualified(path) => path
                .segments
                .last()
                .map(|s| s.as_str())
                .unwrap_or(path.root.as_str()),
            TypeExpr::Literal(lit, _) => match lit {
                Literal::String(s) => s.as_str(),
                Literal::Int(_) => "__literal_int",
                Literal::Float(_) => "__literal_float",
                Literal::Number(_) => "__literal_number",
            },
            TypeExpr::Pointer(inner) => inner.value.surface_mangle_name(),
            TypeExpr::Ref { inner, .. } => inner.value.surface_mangle_name(),
            TypeExpr::Unit => "()",
            TypeExpr::InRange { .. } => "__in_range",
        }
    }
}

impl Expr {
    /// `for` loop patterns are a subset of expressions:
    /// - a simple identifier (`x` → `Expr::FnCall` with an empty path tail), or
    /// - a parenthesized list of identifiers (`(a, b)` → `Expr::TupleLit`, or `(x)` → same shape as `x`).
    ///
    /// Returns `None` if the expression is not a valid `for` binder (e.g. a call or literal).
    pub fn for_loop_pattern_names(&self) -> Option<Vec<Intern<String>>> {
        match self {
            Expr::FnCall(FnCall { path, args: None }) if path.segments.is_empty() => {
                Some(vec![path.root])
            }
            Expr::TupleLit(elems) => {
                let mut out = Vec::with_capacity(elems.len());
                for Typed { value, .. } in elems {
                    let Expr::FnCall(FnCall { path, args: None }) = value else {
                        return None;
                    };
                    if !path.segments.is_empty() {
                        return None;
                    }
                    out.push(path.root);
                }
                Some(out)
            }
            _ => None,
        }
    }

    /// When the `for` pattern binds exactly one name, return it (used by flow analysis).
    pub fn for_loop_single_binding(&self) -> Option<Intern<String>> {
        let names = self.for_loop_pattern_names()?;
        (names.len() == 1).then_some(names[0])
    }
}

/// Extension trait providing [`str::is_pattern_wildcard`].
pub trait PatternNameExt {
    fn is_pattern_wildcard(&self) -> bool;
}

impl PatternNameExt for str {
    fn is_pattern_wildcard(&self) -> bool {
        self == "_"
    }
}

impl Expr {
    pub fn denotes_variant_name(&self, variant: &str) -> bool {
        match self {
            Expr::AnonymousTag(name) => name.as_str() == variant,
            Expr::Lit(Literal::String(value)) => value == variant,
            Expr::TagCall(call) => call.name.as_str() == variant,
            _ => false,
        }
    }
}


impl Expr {
    /// Check if a variant word appears at `byte_pos` within this expression tree.
    fn pattern_variant_word_in(&self, span_table: &SpanTable, byte_pos: usize, word: &str) -> bool {
        match self {
            Expr::When(w) => w.pattern_variant_word_in(span_table, byte_pos, word),
            Expr::If(i) => i.pattern.as_ref().is_some_and(|pattern| {
                pattern
                    .value
                    .variant_word_at(pattern.span_id, span_table, byte_pos, word)
            }),
            Expr::Bind(b) => b.value.pattern_variant_word_in(span_table, byte_pos, word),
            Expr::Binary(b) => {
                b.lhs
                    .value
                    .pattern_variant_word_in(span_table, byte_pos, word)
                    || b.rhs
                        .value
                        .pattern_variant_word_in(span_table, byte_pos, word)
            }
            Expr::FnCall(c) => c
                .args
                .as_ref()
                .map(|args| {
                    args.iter()
                        .any(|a| a.value.pattern_variant_word_in(span_table, byte_pos, word))
                })
                .unwrap_or(false),
            Expr::TagCall(tc) => tc
                .args
                .iter()
                .any(|a| a.value.pattern_variant_word_in(span_table, byte_pos, word)),
            Expr::TupleLit(elems) | Expr::List(elems) => elems
                .iter()
                .any(|e| e.value.pattern_variant_word_in(span_table, byte_pos, word)),
            _ => false,
        }
    }
}

impl WhenExpr {
    /// Check if a variant word appears at `byte_pos` within this `when` expression.
    fn pattern_variant_word_in(&self, span_table: &SpanTable, byte_pos: usize, word: &str) -> bool {
        if let Some(subject) = &self.subject
            && subject
                .value
                .pattern_variant_word_in(span_table, byte_pos, word)
        {
            return true;
        }
        self.arms.iter().any(|arm| match arm {
            WhenArm::Is { pattern, .. } => {
                pattern
                    .value
                    .variant_word_at(pattern.span_id, span_table, byte_pos, word)
            }
            WhenArm::Cond {
                condition, body, ..
            } => {
                condition
                    .value
                    .pattern_variant_word_in(span_table, byte_pos, word)
                    || body
                        .value
                        .pattern_variant_word_in(span_table, byte_pos, word)
            }
            WhenArm::Else(body, _) => body
                .value
                .pattern_variant_word_in(span_table, byte_pos, word),
        })
    }
}

impl BindValue {
    /// Check if a variant word appears at `byte_pos` within this bind value.
    fn pattern_variant_word_in(&self, span_table: &SpanTable, byte_pos: usize, word: &str) -> bool {
        match self {
            BindValue::Expr(e) => e.value.pattern_variant_word_in(span_table, byte_pos, word),
            BindValue::Body { exprs, ret } => {
                exprs
                    .iter()
                    .any(|e| e.value.pattern_variant_word_in(span_table, byte_pos, word))
                    || ret.value.as_ref().is_some_and(|r| {
                        r.value.pattern_variant_word_in(span_table, byte_pos, word)
                    })
            }
            BindValue::Extern | BindValue::Unassigned => false,
        }
    }
}

impl FileAst {
    /// Cursor on a union variant used as an expression (`then False`, bare `True`).
    pub fn expr_union_variant_at_byte(&self, source: &str, byte_pos: usize) -> Option<String> {
        let word = self
            .word_at_byte(byte_pos, source)
            .or_else(|| source.word_at_byte_offset(byte_pos))?;
        let (expr, _) = self.expr_at_byte(byte_pos)?;
        match expr {
            Expr::AnonymousTag(name) if name.as_str() == word => Some(word),
            Expr::TagCall(tc) if tc.name.as_str() == word => Some(word),
            _ => None,
        }
    }

    /// Pattern, expression, or tag-declare site for a union variant literal.
    ///
    /// Detects variants inside `when`/`if is` patterns, expression sites, and
    /// tag declarations (`Comparison is Ordering or Incomparable`). This ensures
    /// goto-definition on a bare-tag variant name resolves to the variant
    /// definition rather than a coincidentally-named type.
    pub fn union_variant_reference_at_byte(&self, source: &str, byte_pos: usize) -> Option<String> {
        self.pattern_variant_literal_at_byte(source, byte_pos)
            .or_else(|| self.expr_union_variant_at_byte(source, byte_pos))
            .or_else(|| self.variant_word_in_tag_declare(byte_pos))
    }

    /// If `byte_pos` is on a union variant literal in an `is` pattern, return its name.
    pub fn pattern_variant_literal_at_byte(&self, source: &str, byte_pos: usize) -> Option<String> {
        let word = self
            .word_at_byte(byte_pos, source)
            .or_else(|| source.word_at_byte_offset(byte_pos))?;
        if self.pattern_variant_word_in(byte_pos, &word) {
            Some(word)
        } else {
            None
        }
    }
}

/// Check if `byte_pos` falls within a variant name inside a tag's union
/// declaration, e.g. `Ordering` in `Comparison is Ordering or Incomparable`.
impl FileAst {
    /// Check if `byte_pos` falls within a variant name inside a tag's union
    /// declaration, e.g. `Ordering` in `Comparison is Ordering or Incomparable`.
    fn variant_word_in_tag_declare(&self, byte_pos: usize) -> Option<String> {
        let span_table = &self.span_table;
        for decl in self.tags.values() {
            let DeclareValue::Union { variants } = &decl.value else {
                continue;
            };
            for v in variants {
                let name = match &v.shape().value {
                    Pattern::Nominal(n, span_id) if span_table.contains(*span_id, byte_pos) => {
                        Some(n.as_str().to_string())
                    }
                    Pattern::Generic { name, span, .. }
                        if span_table.contains(*span, byte_pos) =>
                    {
                        Some(name.as_str().to_string())
                    }
                    _ => None,
                };
                if let Some(name) = name {
                    return Some(name);
                }
            }
        }
        None
    }

    /// Check if a variant word appears at `byte_pos` anywhere in this file.
    fn pattern_variant_word_in(&self, byte_pos: usize, word: &str) -> bool {
        let span_table = &self.span_table;
        for (_, bind) in &self.defs {
            if bind
                .value
                .pattern_variant_word_in(span_table, byte_pos, word)
            {
                return true;
            }
        }
        for (expr, _) in &self.exprs {
            if expr.pattern_variant_word_in(span_table, byte_pos, word) {
                return true;
            }
        }
        false
    }
}

impl Pattern {
    /// Names bound by an `is` pattern's generic parameter keys, e.g. `Some(v)` -> `[v]`.
    /// Wildcards (`_`) are excluded. Nominal and qualified variants bind no names.
    pub fn binding_names(&self) -> Vec<Intern<String>> {
        self.binding_names_filtered(true)
    }

    /// Collect binding names from a pattern, optionally excluding wildcards.
    fn binding_names_filtered(&self, exclude_wildcards: bool) -> Vec<Intern<String>> {
        let mut names = match self {
            Pattern::Generic { params, .. } => params.iter().map(|(k, _)| *k).collect(),
            Pattern::Nominal(name, _)
                if name
                    .as_str()
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase()) =>
            {
                vec![*name]
            }
            Pattern::Nominal(..)
            | Pattern::Qualified(_)
            | Pattern::Literal(..)
            | Pattern::Pointer(_)
            | Pattern::Ref { .. }
            | Pattern::Unit
            | Pattern::InRange { .. }
            | Pattern::ListEmpty => Vec::new(),
            Pattern::ListCons { head, tail } => {
                let mut names = head.value.binding_names_filtered(exclude_wildcards);
                names.extend(tail.value.binding_names_filtered(exclude_wildcards));
                names
            }
            Pattern::Tuple(elems) => elems
                .iter()
                .flat_map(|e| e.value.binding_names_filtered(exclude_wildcards))
                .collect(),
        };
        if exclude_wildcards {
            names.retain(|n| !n.is_pattern_wildcard());
        }
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HashFloat;
    use crate::parameter::ParameterKind;
    use crate::path::ModPath;
    use crate::span::{SpanId, Spanned};
    use crate::ty::Ty;
    use std::collections::HashMap;

    fn intern(s: &str) -> Intern<String> {
        Intern::new(s.to_owned())
    }

    fn simple_var(name: &str) -> Expr {
        let n = intern(name);
        Expr::FnCall(FnCall {
            path: Spanned::new(ModPath::new(n, Vec::new()), SpanId::new(0)),
            args: None,
        })
    }

    fn list_cons_pat(head_name: &str, tail_name: &str) -> Pattern {
        Pattern::ListCons {
            head: Box::new(Spanned {
                value: Pattern::Nominal(intern(head_name), SpanId::new(0)),
                span_id: SpanId::new(0),
            }),
            tail: Box::new(Spanned {
                value: Pattern::Nominal(intern(tail_name), SpanId::new(1)),
                span_id: SpanId::new(1),
            }),
        }
    }

    #[test]
    fn for_loop_pattern_cases() {
        #[allow(clippy::type_complexity)]
        let cases: Vec<(&str, Vec<Expr>, Option<Vec<&str>>, Option<&str>)> = vec![
            (
                "single_name",
                vec![simple_var("i")],
                Some(vec!["i"]),
                Some("i"),
            ),
            (
                "tuple_names",
                vec![Expr::TupleLit(vec![
                    Typed::infer(simple_var("a"), SpanId::new(1)),
                    Typed::infer(simple_var("b"), SpanId::new(2)),
                ])],
                Some(vec!["a", "b"]),
                None,
            ),
            (
                "rejects_calls",
                vec![Expr::FnCall(FnCall {
                    path: Spanned::new(ModPath::new(intern("f"), Vec::new()), SpanId::new(0)),
                    args: Some(vec![]),
                })],
                None,
                None,
            ),
        ];

        for (name, exprs, expected_names, expected_single) in cases {
            let e = exprs.into_iter().next().unwrap();
            let result = e.for_loop_pattern_names();
            let expected: Option<Vec<Intern<String>>> =
                expected_names.map(|v| v.into_iter().map(intern).collect());
            assert_eq!(result, expected, "for_loop_pattern_names({name})");

            let single_result = e.for_loop_single_binding();
            let expected = expected_single.map(intern);
            assert_eq!(single_result, expected, "for_loop_single_binding({name})");
        }
    }

    #[test]
    fn pattern_type_binding_names_cases() {
        let cases: Vec<(&str, Pattern, Vec<&str>)> = vec![
            (
                "generic",
                Pattern::Generic {
                    name: intern("Some"),
                    params: vec![(intern("v"), ParameterKind::Generic)],
                    param_spans: vec![(intern("v"), SpanId::new(0))],
                    span: SpanId::new(0),
                },
                vec!["v"],
            ),
            (
                "excludes_wildcard",
                Pattern::Generic {
                    name: intern("Record"),
                    params: vec![
                        (intern("_"), ParameterKind::Generic),
                        (intern("fields"), ParameterKind::Generic),
                    ],
                    param_spans: vec![
                        (intern("_"), SpanId::new(0)),
                        (intern("fields"), SpanId::new(1)),
                    ],
                    span: SpanId::new(0),
                },
                vec!["fields"],
            ),
            (
                "list_cons_lowercase",
                list_cons_pat("head", "tail"),
                vec!["head", "tail"],
            ),
            (
                "list_cons_uppercase",
                list_cons_pat("Head", "Tail"),
                vec![],
            ),
            ("list_empty", Pattern::ListEmpty, vec![]),
        ];

        for (name, pat, expected_names) in cases {
            let result = pat.binding_names();
            let expected: Vec<Intern<String>> = expected_names.into_iter().map(intern).collect();
            assert_eq!(
                result.len(),
                expected.len(),
                "{name}: expected {} bindings, got {}",
                expected.len(),
                result.len()
            );
            for en in &expected {
                assert!(
                    result.contains(en),
                    "{name}: expected binding `{}` not found in {:?}",
                    en.as_str(),
                    result
                );
            }
        }
    }

    #[test]
    fn surface_mangle_name_cases() {
        let cases: Vec<(&str, TypeExpr)> = vec![
            ("U32", TypeExpr::Nominal(intern("U32"), SpanId::new(1))),
            (
                "__literal_float",
                TypeExpr::Literal(
                    crate::Literal::Float(HashFloat(314.0 / 100.0)),
                    SpanId::new(0),
                ),
            ),
            (
                "__literal_int",
                TypeExpr::Literal(crate::Literal::Int(42), SpanId::new(0)),
            ),
        ];

        for (expected, expr) in cases {
            assert_eq!(
                expr.surface_mangle_name(),
                expected,
                "surface_mangle_name({expr:?})"
            );
        }
    }

    #[test]
    fn pattern_binding_types_list_cons() {
        // pattern: [item, ...rest]
        let pat = list_cons_pat("item", "rest");

        // Subject type: List(NamedTy) = Record { pointer: Ptr(NamedTy), length: ... }
        let named_ty = Ty::Record {
            name: intern("NamedTy"),
            resolved_params: None,
            fields: vec![
                (intern("name"), Box::new(Ty::Opaque(intern("String")))),
                (intern("ty"), Box::new(Ty::Opaque(intern("Type")))),
            ],
        };
        let list_record_fields: Vec<(Intern<String>, Box<Ty>)> = vec![
            (
                intern("pointer"),
                Box::new(Ty::Ptr {
                    inner: Box::new(named_ty.clone()),
                }),
            ),
            (
                intern("length"),
                Box::new(Ty::Opaque(intern("PointerSize"))),
            ),
        ];
        let list_record = Ty::Record {
            name: intern("List"),
            resolved_params: None,
            fields: list_record_fields,
        };
        let mut tag_types = HashMap::new();
        tag_types.insert(intern("List"), list_record);
        let variant_map: crate::ty::VariantMap = HashMap::new();

        let subject_ty = Some(Ty::Record {
            name: intern("List"),
            resolved_params: None,
            fields: vec![
                (
                    intern("pointer"),
                    Box::new(Ty::Ptr {
                        inner: Box::new(named_ty.clone()),
                    }),
                ),
                (
                    intern("length"),
                    Box::new(Ty::Opaque(intern("PointerSize"))),
                ),
            ],
        });

        let bindings = pat.pattern_binding_types(subject_ty.as_ref(), &variant_map, &tag_types);

        assert_eq!(bindings.len(), 2, "should bind both item and rest");

        // item should have the element type (extracted from Ptr)
        let item_ty = bindings.get(&intern("item")).expect("item binding");
        assert_eq!(item_ty, &named_ty, "item should have element type");

        // rest should have List(NamedTy) type
        let rest_ty = bindings.get(&intern("rest")).expect("rest binding");
        match rest_ty {
            Ty::Record { name, fields, .. } => {
                assert_eq!(name.as_str(), "List", "rest should be a List");
                let has_pointer = fields.iter().any(|(n, _)| n.as_str() == "pointer");
                assert!(has_pointer, "rest List should have pointer field");
            }
            other => panic!("rest should be Record, got {:?}", other),
        }
    }
}
