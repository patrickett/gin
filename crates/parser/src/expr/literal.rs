use crate::cursor::TokenCursor;
use crate::unescape::UnescapeExt;
use ast::Spanned;
use ast::{HashFloat, Literal};

use lexer::Token;

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn parse_literal(&mut self) -> Option<Spanned<Literal>> {
        // Peek first to avoid checkpoint/rewind overhead on non-literal tokens.
        match self.peek()? {
            Token::Int(_) | Token::Float(_) | Token::String(_) | Token::UnterminatedString(_) => {}
            _ => return None,
        }

        // We know it's a literal token, so advance and extract the value.
        let span = self.peek_span()?;
        let (token, _span) = self.advance()?;

        let lit = match token {
            Token::Int(n) => Literal::Int(n),
            Token::Float(f) => Literal::Float(HashFloat(f)),
            Token::String(s) => Literal::String(s.unescape()),
            Token::UnterminatedString(s) => Literal::String(s.unescape()),
            // Safety: we already verified the token kind via peek above.
            _ => unreachable!(),
        };

        Some(Spanned {
            value: lit,
            span_id: span,
        })
    }
}
