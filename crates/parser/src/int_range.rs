//! Shared `N...M` literal parsing for declare and type-position `in`.

use ast::{InRangeBounds, TypeExpr};
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
        self.error("parse-expected-type-or-range", "expected integer range or type name after 'in'", span);
        None
    }
}
