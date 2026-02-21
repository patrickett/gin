//! The lexer is handled by [logos](https://github.com/maciejhirsz/logos)

// TODO: replace logos lexer with handwritten for better performance
// PERF: Handwritten lexer can be optimized for specific Gin syntax patterns
// we can assume lowercase word is id and first letter upper is tag

mod semantic_token_type;
mod token;

use chumsky::span::SimpleSpan;
use logos::{Lexer, Logos};

pub use semantic_token_type::*;
pub use token::{LexContext, MAX_INDENT_DEPTH, Token, handle_newline};

pub struct GinLexer<'src> {
    pub inner: Lexer<'src, Token<'src>>,
}

impl<'src> GinLexer<'src> {
    pub fn new(src: &'src str) -> Self {
        Self {
            inner: Token::lexer_with_extras(src, LexContext::default()),
        }
    }

    #[inline(always)]
    fn next_with_indent(&mut self) -> Option<(Token<'src>, SimpleSpan)> {
        if self.inner.extras.pending_dedents > 0 {
            self.inner.extras.pending_dedents -= 1;
            let span: SimpleSpan = self.inner.span().into();
            return Some((Token::Indent, span));
        }
        if self.inner.extras.pending_indent {
            self.inner.extras.pending_indent = false;
            let span: SimpleSpan = self.inner.span().into();
            return Some((Token::Indent, span));
        }

        loop {
            let start = self.inner.span().end;
            let next = self.inner.next()?;
            let end = self.inner.span().end;
            let span: SimpleSpan = (start..end).into();

            match next {
                Ok(tok) => return Some((tok, span)),
                Err(()) => {
                    // Skip one Unicode scalar value. `leading_ones()` on a
                    // UTF-8 leading byte gives the byte-width of the sequence.
                    // If the remainder is empty after a failed match, treat it
                    // as EOF rather than attempting a zero-length bump.
                    let skip = match self.inner.remainder().as_bytes().first() {
                        None => break None,
                        Some(&b) if b.is_ascii() => 1,
                        Some(&b) => b.leading_ones() as usize,
                    };
                    self.inner.bump(skip);
                }
            }
        }
    }

    fn next_raw(&mut self) -> Option<(Token<'src>, SimpleSpan)> {
        if let Some(item) = self.next_with_indent() {
            return Some(item);
        }

        // EOF: flush remaining open indent levels as Dedents.
        let dedent_count = self.inner.extras.indent_depth as usize - 1;
        if dedent_count > 0 {
            self.inner.extras.indent_depth = 1;
            self.inner.extras.pending_dedents = dedent_count as u8;
            self.next_with_indent()
        } else {
            None
        }
    }
}

impl<'src> Iterator for GinLexer<'src> {
    type Item = (Token<'src>, SimpleSpan);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let item = self.next_raw()?;
            if !matches!(item.0, Token::Comment(_) | Token::DocComment(_)) {
                return Some(item);
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Estimate ~4 bytes per token so that `collect()` pre-allocates once.
        let pending = self.inner.extras.pending_dedents as usize
            + usize::from(self.inner.extras.pending_indent);
        let from_source = self.inner.remainder().len() / 4;
        let estimate = from_source + pending;
        (estimate, Some(estimate + estimate / 2))
    }
}
