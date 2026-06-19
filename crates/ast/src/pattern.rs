//! Unified surface (patterns, type-shaped positions, values):
//!
//! - **Values** — [`Expr`] in `crates/ast/src/expr/mod.rs`.
//! - **For binders** — [`Expr`] on [`crate::expr::ForInLoop`] (`for_loop_pattern_names`).
//! - **`if` / `when` `is` patterns** — structural [`TypeExpr`] (`Nominal` / `Qualified` / `Generic`).
//! - **Bind return / receiver** — same structural [`TypeExpr`].
//!
//! The parser builds these [`TypeExpr`] nodes directly (`parser::tag::parse_type_expr`).
//!
//! Ref: https://matklad.github.io/2025/08/09/zigs-lovely-syntax.html#Everything-Is-an-Expression

use internment::Intern;

use crate::TypeExpr;
use crate::expr::{Expr, FnCall, Literal, Typed, WhenArm, WhenExpr};
use crate::parameter::ParameterKind;
use crate::source::SourceExt;
use crate::span::{SpanId, SpanTable};
use crate::{BindValue, DeclareValue, FileAst};

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
            TypeExpr::ListEmpty => "[]",
            TypeExpr::ListCons { .. } => "[..]",
            TypeExpr::Tuple(_) => "(..)",
        }
    }
}

impl Expr {
    /// Extract the literal value from a union variant shape (if it is a literal).
    pub fn literal_value(&self) -> Option<crate::Literal> {
        match self {
            Expr::Lit(lit) => Some(lit.clone()),
            _ => None,
        }
    }

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

impl TypeExpr {
    /// True when a tagged pattern argument is a union variant literal (`False`, `'x86_64'`), not a binder.
    pub fn denotes_variant_name(&self, variant: &str) -> bool {
        match self {
            TypeExpr::Nominal(n, _) => n.as_str() == variant,
            TypeExpr::Literal(Literal::String(s), _) => s == variant,
            TypeExpr::Generic { name, params, .. } => {
                name.as_str() == variant
                    && params.iter().all(|(n, k)| {
                        n.is_pattern_wildcard() || matches!(k, ParameterKind::Generic)
                    })
            }
            _ => false,
        }
    }
}

impl TypeExpr {
    /// Check if `byte_pos` is on a variant word within this type expression pattern.
    fn variant_word_at(
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
            TypeExpr::Generic {
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
                param_spans
                    .iter()
                    .enumerate()
                    .any(|(slot, (pname, pspan))| {
                        let ps = span_table.get(*pspan);
                        if ps.start() <= byte_pos
                            && byte_pos < ps.end()
                            && (word == pname.as_str()
                                || (pname.is_pattern_wildcard() && word == "_"))
                        {
                            let kind = params.get(slot).map(|(_, k)| k);
                            if let Some(ParameterKind::Tagged(sp)) = kind {
                                return sp.value.denotes_variant_name(word);
                            }
                        }
                        false
                    })
            }
            TypeExpr::ListCons { head, tail } => {
                head.value
                    .variant_word_at(pattern_span_id, span_table, byte_pos, word)
                    || tail
                        .value
                        .variant_word_at(pattern_span_id, span_table, byte_pos, word)
            }
            TypeExpr::Tuple(elems) => elems.iter().any(|e| {
                e.value
                    .variant_word_at(pattern_span_id, span_table, byte_pos, word)
            }),
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
                    TypeExpr::Nominal(n, span_id) if span_table.contains(*span_id, byte_pos) => {
                        Some(n.as_str().to_string())
                    }
                    TypeExpr::Generic { name, span, .. }
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

impl TypeExpr {
    /// Names bound by an `is <type>` pattern (`TypeGeneric` parameter keys), e.g. `Some(v)` → `[v]`.
    /// Wildcards (`_`) are excluded. [`TypeExpr::Nominal`] and [`TypeExpr::Qualified`] bind no names.
    pub fn binding_names(&self) -> Vec<Intern<String>> {
        self.binding_names_filtered(true)
    }
}

impl TypeExpr {
    /// Collect binding names from a pattern, optionally excluding wildcards.
    fn binding_names_filtered(&self, exclude_wildcards: bool) -> Vec<Intern<String>> {
        let mut names = match self {
            TypeExpr::Generic { params, .. } => params.iter().map(|(k, _)| *k).collect(),
            TypeExpr::Nominal(name, _)
                if name
                    .as_str()
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase()) =>
            {
                vec![*name]
            }
            TypeExpr::Nominal(..)
            | TypeExpr::Qualified(_)
            | TypeExpr::Literal(..)
            | TypeExpr::Pointer(_)
            | TypeExpr::Ref { .. }
            | TypeExpr::Unit
            | TypeExpr::InRange { .. }
            | TypeExpr::ListEmpty => Vec::new(),
            TypeExpr::ListCons { head, tail } => {
                let mut names = head.value.binding_names_filtered(exclude_wildcards);
                names.extend(tail.value.binding_names_filtered(exclude_wildcards));
                names
            }
            TypeExpr::Tuple(elems) => elems
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

    fn list_cons_pat(head_name: &str, tail_name: &str) -> TypeExpr {
        TypeExpr::ListCons {
            head: Box::new(Spanned {
                value: TypeExpr::Nominal(intern(head_name), SpanId::new(0)),
                span_id: SpanId::new(0),
            }),
            tail: Box::new(Spanned {
                value: TypeExpr::Nominal(intern(tail_name), SpanId::new(1)),
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
        let cases: Vec<(&str, TypeExpr, Vec<&str>)> = vec![
            (
                "generic",
                TypeExpr::Generic {
                    name: intern("Some"),
                    params: vec![(intern("v"), ParameterKind::Generic)],
                    param_spans: vec![(intern("v"), SpanId::new(0))],
                    span: SpanId::new(0),
                },
                vec!["v"],
            ),
            (
                "excludes_wildcard",
                TypeExpr::Generic {
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
            ("list_cons_uppercase", list_cons_pat("Head", "Tail"), vec![]),
            ("list_empty", TypeExpr::ListEmpty, vec![]),
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
                TypeExpr::Literal(crate::Literal::Float(HashFloat(3.14)), SpanId::new(0)),
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
            fields: list_record_fields,
        };
        let mut tag_types = HashMap::new();
        tag_types.insert(intern("List"), list_record);
        let variant_map: crate::ty::VariantMap = HashMap::new();

        let subject_ty = Some(Ty::Record {
            name: intern("List"),
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
            Ty::Record { name, fields } => {
                assert_eq!(name.as_str(), "List", "rest should be a List");
                let has_pointer = fields.iter().any(|(n, _)| n.as_str() == "pointer");
                assert!(has_pointer, "rest List should have pointer field");
            }
            other => panic!("rest should be Record, got {:?}", other),
        }
    }
}
