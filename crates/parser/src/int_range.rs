//! Shared `N...M` literal parsing for declare and type-position `in`.

use ast::{Expr, InRangeBounds, Literal, Range, Spanned, TypeExpr, Typed};
use i256::I256;
use lexer::Token;

use crate::cursor::TokenCursor;

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn parse_signed_int(&mut self) -> Option<I256> {
        let neg = self.eat(&Token::Minus);
        match self.peek()? {
            &Token::Int(n) => {
                self.advance();
                if neg {
                    Some(-I256::from_u128(n))
                } else {
                    Some(I256::from_u128(n))
                }
            }
            _ => None,
        }
    }

    pub fn parse_int_range(&mut self) -> Option<(I256, I256)> {
        let start = self.parse_signed_int()?;
        if !self.eat(&Token::Infer) {
            return None;
        }
        let end = self.parse_signed_int()?;
        Some((start, end))
    }

    /// Parse `in N...M` or `in Tag` after `in` was consumed.
    pub fn parse_in_range_type(&mut self) -> Option<TypeExpr> {
        let span = self.current_span();
        if let Some((min, max)) = self.parse_int_range() {
            return Some(TypeExpr::InRange {
                bounds: InRangeBounds::Literal(min, max),
                span,
            });
        }
        if let Some(Token::Tag(t)) = self.peek() {
            let name = self.intern(t);
            self.advance();
            return Some(TypeExpr::InRange {
                bounds: InRangeBounds::Tag(name),
                span,
            });
        }
        if let Some(Token::Id(t)) = self.peek() {
            let name = self.intern(t);
            self.advance();
            return Some(TypeExpr::InRange {
                bounds: InRangeBounds::Tag(name),
                span,
            });
        }
        self.error(
            "parse-expected-type-or-range",
            "expected integer range or type name after 'in'",
            span,
        );
        None
    }

    pub fn parse_in_range_expr(&mut self) -> Option<Spanned<Expr>> {
        let span = self.current_span();
        let bounds = if let Some((min, max)) = self.parse_int_range() {
            InRangeBounds::Literal(min, max)
        } else if let Some(Token::Tag(t)) = self.peek() {
            let name = self.intern(t);
            self.advance();
            InRangeBounds::Tag(name)
        } else if let Some(Token::Id(t)) = self.peek() {
            let name = self.intern(t);
            self.advance();
            InRangeBounds::Tag(name)
        } else {
            self.error(
                "parse-expected-type-or-range",
                "expected integer range or type name after 'in'",
                span,
            );
            return None;
        };
        Some(Spanned {
            value: in_range_expr(bounds, span),
            span_id: span,
        })
    }
}

fn in_range_expr(bounds: InRangeBounds, span: ast::SpanId) -> Expr {
    let (start, end) = match bounds {
        InRangeBounds::Literal(min, max) => (
            Expr::Lit(Literal::Int(min.as_u128())),
            Expr::Lit(Literal::Int(max.as_u128())),
        ),
        InRangeBounds::Tag(name) => {
            let tag = Expr::AnonymousTag(name);
            (tag.clone(), tag)
        }
    };
    Expr::Range(Range::new(
        Typed::infer(start, span),
        Typed::infer(end, span),
    ))
}
