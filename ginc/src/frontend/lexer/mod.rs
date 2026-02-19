//! The lexer is handled by [logos](https://github.com/maciejhirsz/logos)

// TODO: replace logos lexer with handwritten for better performance
// PERF: Handwritten lexer can be optimized for specific Gin syntax patterns
// we can assume lowercase word is id and first letter upper is tag

mod semantic_token_type;
mod token;
mod tokenize;

use chumsky::span::SimpleSpan;
use logos::{Lexer, Logos};

pub use semantic_token_type::*;
pub use token::*;
pub use tokenize::*;

pub const MAX_INDENT_DEPTH: usize = 16;

/// Indent and Dedent are mutually exclusive branches per newline.
/// We can exploit this by using `pending_indent` and `pending_dedents`
#[derive(Debug)]
pub struct LexContext {
    indent_stack: [u16; MAX_INDENT_DEPTH],
    indent_depth: u8,
    /// Dedent tokens still to be emitted, decremented one per call.
    pending_dedents: u8,
    /// Whether a single Indent token is waiting to be emitted.
    pending_indent: bool,
}

impl Default for LexContext {
    fn default() -> Self {
        Self {
            indent_stack: [0u16; MAX_INDENT_DEPTH],
            indent_depth: 1,
            pending_dedents: 0,
            pending_indent: false,
        }
    }
}

#[inline]
fn handle_newline<'src>(lex: &mut Lexer<'src, Token<'src>>) -> Token<'src> {
    // `bytes` and `indent` are tracked separately so that a tab (1 byte, 4 columns)
    // does not cause `lex.bump` to skip real source characters.

    let remainder = lex.remainder().as_bytes();
    let mut indent = 0u16;
    let mut bytes = 0usize;
    for &b in remainder {
        match b {
            b' ' => {
                indent += 1;
                bytes += 1;
            }
            b'\t' => {
                indent += 4;
                bytes += 1;
            }
            _ => break,
        }
    }
    lex.bump(bytes);

    let extras = &mut lex.extras;
    let depth = extras.indent_depth as usize;
    let current = extras.indent_stack[depth - 1];

    if indent > current {
        if depth < MAX_INDENT_DEPTH {
            extras.indent_stack[depth] = indent;
            extras.indent_depth += 1;
            extras.pending_indent = true;
        }
        // Silently cap at MAX_INDENT_DEPTH; parser continues at the deepest
        // valid level. Callers that need to detect overflow can inspect the
        // token stream for unexpected structure.
    } else if indent < current {
        // Walk the stack with a local, write indent_depth once at the end.
        let mut d = depth;
        while extras.indent_stack[d - 1] > indent {
            d -= 1;
        }
        extras.pending_dedents = (depth - d) as u8;
        extras.indent_depth = d as u8;
    }

    Token::Newline
}

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
        // Pending synthetic tokens are rare; the CPU branch predictor handles these
        // well after the first few tokens of any real file.
        if self.inner.extras.pending_dedents > 0 {
            self.inner.extras.pending_dedents -= 1;
            let span: SimpleSpan = self.inner.span().into();
            return Some((Token::Dedent, span));
        }
        if self.inner.extras.pending_indent {
            self.inner.extras.pending_indent = false;
            let span: SimpleSpan = self.inner.span().into();
            return Some((Token::Indent, span));
        }

        // Hot path: read real tokens. Invalid bytes are skipped without
        // recursion and without eprintln (parser handles error reporting).
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
}

impl<'src> Iterator for GinLexer<'src> {
    type Item = (Token<'src>, SimpleSpan);

    fn next(&mut self) -> Option<Self::Item> {
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

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Estimate ~4 bytes per token so that `collect()` pre-allocates once.
        let pending = self.inner.extras.pending_dedents as usize
            + usize::from(self.inner.extras.pending_indent);
        let from_source = self.inner.remainder().len() / 4;
        let estimate = from_source + pending;
        (estimate, Some(estimate + estimate / 2))
    }
}
