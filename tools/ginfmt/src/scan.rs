//! Token scanning with the same treatment of `"` format strings as `parser`, which reads
//! their text from the source instead of lexing it. The lexer's own tokens are used for
//! everything else, so comments, newlines and unknown characters stay visible.

use lexer::{Lexer, TokenKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tok {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

impl Tok {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }

    pub fn is_comment(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Comment | TokenKind::DocComment | TokenKind::ModuleDocComment
        )
    }
}

/// The end of the `"` string that opens at `start`: just past the closing quote, or the end of
/// the line. Mirrors `parser`'s scan: `\"` and `\\` are escapes, `\(` opens a hole.
fn format_string_end(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut at = start + 1;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            b'"' => return at + 1,
            b'\n' => break,
            b'\\' => match bytes.get(at + 1) {
                Some(b'"' | b'\\' | b'(') => at += 2,
                _ => at += 1,
            },
            _ => at += 1,
        }
    }
    at
}

/// A format string is returned as one token of kind `FormatStringDelim`.
pub fn scan(source: &str) -> Vec<Tok> {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();
    while let Some(token) = lexer.next_raw() {
        let (start, end) = (token.span.start(), token.span.end());
        if token.kind == TokenKind::FormatStringDelim {
            let end = format_string_end(source, start);
            lexer.skip_to(end);
            tokens.push(Tok { kind: token.kind, start, end });
        } else {
            tokens.push(Tok { kind: token.kind, start, end });
        }
    }
    tokens
}
