// Unified surface (patterns, type-shaped positions, values) — **done for AST boundaries**:
//
// - **Values** — [`Expr`] in `crates/ast/src/expr/mod.rs`.
// - **For binders** — [`Expr`] on [`crate::expr::ForInLoop`] (`for_loop_pattern_names`).
// - **`if` / `when` `is` patterns** — [`Expr::IsPattern`] (`is_pattern_expr_from_tag`).
// - **Bind return / receiver tags** — [`Expr::TypeTag`] (`type_tag_expr_from_tag`).
//
// [`Tag`] remains the *payload* inside `IsPattern` / `TypeTag` and for declare bodies,
// `ParameterKind::Tagged`, etc., until generic type parameters are fully expression-shaped
// (then `Tag` can shrink toward a pure typeck IR).
//
// Ref: https://matklad.github.io/2025/08/09/zigs-lovely-syntax.html#Everything-Is-an-Expression

use internment::Intern;

use crate::expr::{Expr, FnCall};
use crate::span::Spanned;
use crate::tag::Tag;

/// `for` loop patterns are a subset of expressions:
/// - a simple identifier (`x` → `Expr::FnCall` with an empty path tail), or
/// - a parenthesized list of identifiers (`(a, b)` → `Expr::TupleLit`, or `(x)` → same shape as `x`).
///
/// Returns `None` if the expression is not a valid `for` binder (e.g. a call or literal).
pub fn for_loop_pattern_names(pat: &Expr) -> Option<Vec<Intern<String>>> {
    match pat {
        Expr::FnCall(FnCall { path, args: None }) if path.segments.is_empty() => {
            Some(vec![path.root])
        }
        Expr::TupleLit(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for Spanned(e, _) in elems {
                let Expr::FnCall(FnCall { path, args: None }) = e else {
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
pub fn for_loop_single_binding(pat: &Expr) -> Option<Intern<String>> {
    let names = for_loop_pattern_names(pat)?;
    (names.len() == 1).then_some(names[0])
}

/// Wrap a parsed `is`-pattern [`Tag`] as the unified [`Expr`] surface ([`Expr::IsPattern`]).
#[inline]
pub fn is_pattern_expr_from_tag(tag: Tag) -> Spanned<Expr> {
    let span = tag.span();
    Spanned(Expr::IsPattern(Box::new(tag)), span)
}

/// If `expr` is [`Expr::IsPattern`], return the inner tag.
#[inline]
pub fn is_pattern_as_tag(expr: &Expr) -> Option<&Tag> {
    match expr {
        Expr::IsPattern(t) => Some(t),
        _ => None,
    }
}

/// Wrap a tag from a **type** position (return type, receiver) as [`Expr::TypeTag`].
#[inline]
pub fn type_tag_expr_from_tag(tag: Tag) -> Spanned<Expr> {
    let span = tag.span();
    Spanned(Expr::TypeTag(Box::new(tag)), span)
}

#[inline]
pub fn type_tag_as_tag(expr: &Expr) -> Option<&Tag> {
    match expr {
        Expr::TypeTag(t) => Some(t),
        _ => None,
    }
}

/// [`Expr::IsPattern`] or [`Expr::TypeTag`] — both wrap a [`Tag`] on the unified `Expr` surface.
#[inline]
pub fn tag_surface_as_tag(expr: &Expr) -> Option<&Tag> {
    match expr {
        Expr::IsPattern(t) | Expr::TypeTag(t) => Some(t),
        _ => None,
    }
}

/// Names bound by an `is <tag>` pattern ([`Tag::Generic`] parameter keys), e.g. `Some(v)` → `[v]`.
/// [`Tag::Nominal`] and [`Tag::Qualified`] bind no names.
pub fn tag_pattern_binding_names(tag: &Tag) -> Vec<Intern<String>> {
    match tag {
        Tag::Generic(_, params, _) => params.keys().copied().collect(),
        Tag::Nominal(_, _) | Tag::Qualified(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parameter::ParameterKind;
    use crate::path::ModPath;
    use crate::span::SpanId;
    use indexmap::IndexMap;

    fn intern(s: &str) -> Intern<String> {
        Intern::new(s.to_owned())
    }

    fn simple_var(name: &str) -> Expr {
        let n = intern(name);
        Expr::FnCall(FnCall {
            path: ModPath::new(n, Vec::new(), SpanId::new(0)),
            args: None,
        })
    }

    #[test]
    fn for_loop_pattern_single_name() {
        let e = simple_var("i");
        assert_eq!(for_loop_pattern_names(&e), Some(vec![intern("i")]));
        assert_eq!(for_loop_single_binding(&e), Some(intern("i")));
    }

    #[test]
    fn for_loop_pattern_tuple_names() {
        let e = Expr::TupleLit(vec![
            Spanned(simple_var("a"), SpanId::new(1)),
            Spanned(simple_var("b"), SpanId::new(2)),
        ]);
        assert_eq!(
            for_loop_pattern_names(&e),
            Some(vec![intern("a"), intern("b")])
        );
        assert_eq!(for_loop_single_binding(&e), None);
    }

    #[test]
    fn for_loop_pattern_rejects_calls() {
        let n = intern("f");
        let e = Expr::FnCall(FnCall {
            path: ModPath::new(n, Vec::new(), SpanId::new(0)),
            args: Some(vec![]),
        });
        assert_eq!(for_loop_pattern_names(&e), None);
    }

    #[test]
    fn tag_pattern_binding_names_generic() {
        let mut params = IndexMap::new();
        params.insert(intern("payload"), ParameterKind::Generic);
        let tag = Tag::Generic(intern("Some"), params, SpanId::new(0));
        assert_eq!(tag_pattern_binding_names(&tag), vec![intern("payload")]);
    }

    #[test]
    fn tag_pattern_binding_names_nominal_empty() {
        let tag = Tag::Nominal(intern("None"), SpanId::new(0));
        assert!(tag_pattern_binding_names(&tag).is_empty());
    }

    #[test]
    fn is_pattern_roundtrips_tag() {
        let tag = Tag::Nominal(intern("None"), SpanId::new(7));
        let Spanned(e, sp) = is_pattern_expr_from_tag(tag);
        assert_eq!(sp, SpanId::new(7));
        assert!(matches!(
            is_pattern_as_tag(&e),
            Some(Tag::Nominal(n, s)) if *n == intern("None") && *s == SpanId::new(7)
        ));
    }

    #[test]
    fn type_tag_roundtrips_tag() {
        let tag = Tag::Nominal(intern("Str"), SpanId::new(3));
        let Spanned(e, sp) = type_tag_expr_from_tag(tag);
        assert_eq!(sp, SpanId::new(3));
        assert!(matches!(
            type_tag_as_tag(&e),
            Some(Tag::Nominal(n, s)) if *n == intern("Str") && *s == SpanId::new(3)
        ));
        assert!(is_pattern_as_tag(&e).is_none());
        assert!(matches!(
            tag_surface_as_tag(&e),
            Some(Tag::Nominal(n, s)) if *n == intern("Str") && *s == SpanId::new(3)
        ));
    }
}
