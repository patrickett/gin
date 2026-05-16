//! Shared parameter parsing helpers used by both `parse_bind`'s param list and
//! `parse_declare`'s record/declare param list.
//!
//! The two call sites historically diverged: only the declare side accepted the
//! `id id` form (parameter name + lowercase type-variable, e.g. `start x` in
//! `Range[x] has (start x, end x)`). This module centralises the
//! "after-name" classification so both parsers stay in sync.

use ast::{Expr, ParameterKind, Spanned};
use internment::Intern;
use lexer::Token;

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;
use crate::tag::parse_type_expr;

/// Classify the tokens that may follow an already-consumed parameter `name`.
///
/// Recognises (in order):
/// - `id Tag` / `id Tag(args)` → tagged parameter (concrete type)
/// - `id id` → tagged type variable (the second `id` is treated as a type
///   variable name, e.g. `start x`)
/// - `id : expr` → default value
/// - bare `id` → generic parameter
pub fn parse_param_after_name(
    cursor: &mut TokenCursor,
    expr_parser: ExprFn,
    name: Intern<String>,
) -> Option<(Intern<String>, ParameterKind)> {
    if matches!(cursor.peek(), Some(Token::Tag(_))) {
        let sp = parse_type_expr(cursor, expr_parser)?;
        return Some((
            name,
            ParameterKind::Tagged(Box::new(Spanned {
                value: sp.value.into(),
                span_id: sp.span_id,
            })),
        ));
    }

    if let Some(Token::Id(t)) = cursor.peek() {
        let span = cursor.peek_span()?;
        let ty_name = cursor.intern(t);
        cursor.advance();
        let sp = Spanned {
            value: Expr::TypeNominal(ty_name, span),
            span_id: span,
        };
        return Some((name, ParameterKind::Tagged(Box::new(sp))));
    }

    if cursor.eat(&Token::Colon) {
        let expr = expr_parser(cursor);
        return Some((name, ParameterKind::Default(Box::new(expr))));
    }

    Some((name, ParameterKind::Generic))
}
