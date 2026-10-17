//! Shared parsing for `name Type` syntax (no value).
//!
//! Used by record/`has` fields, unassigned binds (`val Cell`), and the same
//! shape on parameters via [`crate::params`].

use ast::{Expr, Spanned};
use lexer::Token;

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;
use crate::int_range::in_range_expr;

impl<'src, 't> TokenCursor<'src, 't> {
    /// Parse a type annotation where the parameter/bind name was already consumed.
    ///
    /// Recognises `in N to M`, `in Tag`, `Tag`, `Tag(...)`, and lowercase `id` type variables (`start x`).
    pub fn parse_type_annotation(&mut self, expr_parser: ExprFn) -> Option<Spanned<Expr>> {
        if self.eat(&Token::In) {
            let (bounds, span) = self.parse_in_range_bounds()?;
            return Some(Spanned {
                value: in_range_expr(bounds, span),
                span_id: span,
            });
        }

        if matches!(
            self.peek(),
            Some(Token::At | Token::Tag(_) | Token::SelfTag | Token::BracketOpen)
        ) {
            let sp = self.parse_type_expr(expr_parser)?;
            return Some(Spanned {
                value: sp.value,
                span_id: sp.span_id,
            });
        }

        if let Some(Token::Id(t)) = self.peek() {
            let span = self.peek_span()?;
            let ty_name = self.intern(t);
            self.advance();
            return Some(Spanned {
                value: Expr::AnonymousTag(ty_name),
                span_id: span,
            });
        }

        None
    }
}
