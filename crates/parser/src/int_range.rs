//! Shared `N to M` literal parsing for declare and type-position `in`.

use ast::{Expr, InRangeBounds, Literal, Range, Typed};
use diagnostic::DiagnosticCode;
use diagnostic::code::{ExpectedSyntaxCode, ParseCode};
use i256::I256;
use lexer::Token;

use crate::cursor::TokenCursor;

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn parse_signed_int(&mut self) -> Option<I256> {
        let neg = self.eat(&Token::Minus);
        match self.peek()? {
            &Token::Int(n) => {
                self.advance();
                if neg { Some(-n) } else { Some(n) }
            }
            _ => None,
        }
    }

    pub fn parse_int_range(&mut self) -> Option<(I256, I256)> {
        let start = self.parse_signed_int()?;
        if !self.eat(&Token::To) {
            return None;
        }
        let end = self.parse_signed_int()?;
        Some((start, end))
    }

    /// Parse `N to M`, `N to Tag`, or `Tag` after `in` was consumed.
    pub fn parse_in_range_bounds(&mut self) -> Option<(InRangeBounds, ast::SpanId)> {
        let span = self.current_span();
        if let Some(min) = self.parse_signed_int() {
            if !self.eat(&Token::To) {
                // parse-expected-type-or-range
                self.error(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::TypeOrRange)),
                    "expected `to` after integer range start",
                    span,
                );
                return None;
            }
            let bounds = if let Some(max) = self.parse_signed_int() {
                InRangeBounds::Literal(min, max)
            } else if let Some(Token::Tag(t) | Token::Id(t)) = self.peek() {
                let max = self.intern(t);
                self.advance();
                InRangeBounds::LiteralToTag(min, max)
            } else {
                // parse-expected-type-or-range
                self.error(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::TypeOrRange)),
                    "expected integer or compile-time name after `to`",
                    span,
                );
                return None;
            };
            return Some((bounds, span));
        }
        if let Some(Token::Tag(t) | Token::Id(t)) = self.peek() {
            let name = self.intern(t);
            self.advance();
            return Some((InRangeBounds::Tag(name), span));
        }
        // parse-expected-type-or-range
        self.error(
            DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::TypeOrRange)),
            "expected integer range or type name after 'in'",
            span,
        );
        None
    }
}

pub(crate) fn in_range_expr(bounds: InRangeBounds, span: ast::SpanId) -> Expr {
    let (start, end) = match bounds {
        InRangeBounds::Literal(min, max) => {
            (Expr::Lit(Literal::Int(min)), Expr::Lit(Literal::Int(max)))
        }
        InRangeBounds::LiteralToTag(min, max) => {
            (Expr::Lit(Literal::Int(min)), Expr::AnonymousTag(max))
        }
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
