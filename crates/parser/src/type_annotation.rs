//! Shared parsing for `name Type` syntax (no value).
//!
//! Used by record/`has` fields, unassigned binds (`val Cell`), and the same
//! shape on parameters via [`crate::params`].

use ast::{Spanned, TypeExpr};
use lexer::Token;

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;

impl<'src, 't> TokenCursor<'src, 't> {
    /// Parse a type annotation where the parameter/bind name was already consumed.
    ///
    /// Recognises `in N...M`, `in Tag`, `Tag`, `Tag(...)`, and lowercase `id` type variables (`start x`).
    pub fn parse_type_annotation(&mut self, expr_parser: ExprFn) -> Option<Spanned<TypeExpr>> {
        if self.eat(&Token::In) {
            let span = self.last_consumed_span();
            let value = self.parse_in_range_type()?;
            return Some(Spanned {
                value,
                span_id: span,
            });
        }

        if matches!(self.peek(), Some(Token::Tag(_))) {
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
                value: TypeExpr::Nominal(ty_name, span),
                span_id: span,
            });
        }

        None
    }
}
