use crate::unescape::unescape;
use ast::Literal;
use ast::Spanned;

use lexer::Token;

use crate::cursor::TokenCursor;

pub fn parse_literal(cursor: &mut TokenCursor) -> Option<Spanned<Literal>> {
    // Peek first to avoid checkpoint/rewind overhead on non-literal tokens.
    match cursor.peek()? {
        Token::Int(_) | Token::Float(_) | Token::String(_) | Token::UnterminatedString(_) => {}
        _ => return None,
    }

    // We know it's a literal token, so advance and extract the value.
    let span = cursor.peek_span()?;
    let (token, _span) = cursor.advance()?;

    let lit = match token {
        Token::Int(n) => Literal::Int(n),
        Token::Float(f) => Literal::Float(f),
        Token::String(s) => Literal::String(unescape(s)),
        Token::UnterminatedString(s) => Literal::String(unescape(s)),
        // Safety: we already verified the token kind via peek above.
        _ => unreachable!(),
    };

    Some(Spanned {
        value: lit,
        span_id: span,
    })
}
