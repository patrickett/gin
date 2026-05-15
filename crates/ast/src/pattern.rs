// Unified surface (patterns, type-shaped positions, values):
//
// - **Values** — [`Expr`] in `crates/ast/src/expr/mod.rs`.
// - **For binders** — [`Expr`] on [`crate::expr::ForInLoop`] (`for_loop_pattern_names`).
// - **`if` / `when` `is` patterns** — structural [`TypeExpr`] (`Nominal` / `Qualified` / `Generic`).
// - **Bind return / receiver** — same structural [`TypeExpr`].
//
// The parser builds these [`TypeExpr`] nodes directly (`parser::tag::parse_type_expr`).
//
// Ref: https://matklad.github.io/2025/08/09/zigs-lovely-syntax.html#Everything-Is-an-Expression

use internment::Intern;

use crate::TypeExpr;
use crate::expr::{Expr, FnCall, Literal};
use crate::span::Spanned;

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
            for Spanned { value, .. } in elems {
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
pub fn for_loop_single_binding(pat: &Expr) -> Option<Intern<String>> {
    let names = for_loop_pattern_names(pat)?;
    (names.len() == 1).then_some(names[0])
}

/// Root name used for mangling `Receiver.method` (nominal / generic name, or qualified last segment).
pub fn type_surface_mangle_name(e: &TypeExpr) -> &str {
    match e {
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
    }
}

/// Extract the literal value from a union variant shape (if it is a literal).
pub fn literal_value_from_expr(e: &Expr) -> Option<crate::Literal> {
    match e {
        Expr::Lit(lit) => Some(lit.clone()),
        _ => None,
    }
}

/// Names bound by an `is <type>` pattern (`TypeGeneric` parameter keys), e.g. `Some(v)` → `[v]`.
/// [`TypeExpr::Nominal`] and [`TypeExpr::Qualified`] bind no names.
pub fn pattern_type_binding_names(expr: &TypeExpr) -> Vec<Intern<String>> {
    match expr {
        TypeExpr::Generic { params, .. } => params.iter().map(|(k, _)| *k).collect(),
        TypeExpr::Nominal(..) | TypeExpr::Qualified(_) | TypeExpr::Literal(..) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parameter::ParameterKind;
    use crate::path::ModPath;
    use crate::span::{SpanId, Spanned};

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

    #[test]
    fn for_loop_pattern_single_name() {
        let e = simple_var("i");
        assert_eq!(for_loop_pattern_names(&e), Some(vec![intern("i")]));
        assert_eq!(for_loop_single_binding(&e), Some(intern("i")));
    }

    #[test]
    fn for_loop_pattern_tuple_names() {
        let e = Expr::TupleLit(vec![
            Spanned {
                value: simple_var("a"),
                span_id: SpanId::new(1),
            },
            Spanned {
                value: simple_var("b"),
                span_id: SpanId::new(2),
            },
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
            path: Spanned::new(ModPath::new(n, Vec::new()), SpanId::new(0)),
            args: Some(vec![]),
        });
        assert_eq!(for_loop_pattern_names(&e), None);
    }

    #[test]
    fn pattern_type_binding_names_generic() {
        let e = TypeExpr::Generic {
            name: intern("Some"),
            params: vec![(intern("v"), ParameterKind::Generic)],
            span: SpanId::new(0),
        };
        assert_eq!(pattern_type_binding_names(&e), vec![intern("v")]);
    }

    #[test]
    fn type_surface_mangle_name_nominal() {
        let e = TypeExpr::Nominal(intern("U32"), SpanId::new(1));
        assert_eq!(type_surface_mangle_name(&e), "U32");
    }
}
