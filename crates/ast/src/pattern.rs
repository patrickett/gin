// TODO: investigate unifying patterns, types (Tag), and expressions (Expr) into a single syntactic category.
//
// Design: Currently Gin has three separate grammatical categories:
//   - Expr (crates/ast/src/expr/mod.rs) — runtime expressions
//   - Tag (crates/ast/src/tag.rs) — type annotations (Nominal, Generic, Qualified)
//   - For-loop binders (this file) — parsed as Expr and validated here / in typeck
//
// Zig's approach is to use the same surface syntax for all three, categorizing
// during semantic analysis rather than parsing. This:
//   1. Reduces combinatorial syntax explosion (types, values, and patterns have
//      the same general tree shape)
//   2. Enables using `if`/`when` in type positions (e.g., `if (true) T else U`)
//   3. Makes generic instantiation look like a plain function call: `Maybe(Int)`
//   4. Simplifies the parser by removing the need for separate parse_tag(),
//      parse_pattern(), and parse_expression() dispatch
//
// Progress: `for` loop headers now store `Spanned<Expr>` (see `ForInLoop`).
// Next steps: type positions as Expr, then fold Tag into Expr with a classifier.
// Ref: https://matklad.github.io/2025/08/09/zigs-lovely-syntax.html#Everything-Is-an-Expression

use internment::Intern;

use crate::expr::{Expr, FnCall};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::ModPath;
    use crate::span::SpanId;

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
}
