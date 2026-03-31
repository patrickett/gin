//! The lexer is handled by [logos](https://github.com/maciejhirsz/logos)

// TODO: replace logos lexer with handwritten for better performance
// PERF: Handwritten lexer can be optimized for specific Gin syntax patterns
// we can assume lowercase word is id and first letter upper is tag

mod format_string;
mod token;

use chumsky::span::SimpleSpan;
use diagnostic::lex::LexSymptom;
use format_string::*;
use logos::{Lexer, Logos};
use std::collections::VecDeque;
pub use token::{LexContext, MAX_INDENT_DEPTH, Token, handle_newline};

pub struct GinLexer<'src> {
    inner: Option<Lexer<'src, Token<'src>>>,
    pub errors: Vec<(LexSymptom, SimpleSpan)>,
    pending: VecDeque<(Token<'src>, SimpleSpan)>,
}

impl<'src> GinLexer<'src> {
    pub fn new(src: &'src str) -> Self {
        Self {
            inner: Some(Token::lexer_with_extras(src, LexContext::default())),
            errors: Vec::new(),
            pending: VecDeque::new(),
        }
    }

    /// Access the inner lexer. Panics if called while morphed (should never happen).
    #[inline(always)]
    fn lexer(&self) -> &Lexer<'src, Token<'src>> {
        self.inner.as_ref().expect("lexer temporarily morphed")
    }

    #[inline(always)]
    fn lexer_mut(&mut self) -> &mut Lexer<'src, Token<'src>> {
        self.inner.as_mut().expect("lexer temporarily morphed")
    }

    #[inline(always)]
    fn next_with_indent(&mut self) -> Option<(Token<'src>, SimpleSpan)> {
        // Drain pending queue first (format string sub-tokens)
        if let Some(item) = self.pending.pop_front() {
            return Some(item);
        }

        let lex = self.lexer_mut();
        if lex.extras.pending_dedents > 0 {
            lex.extras.pending_dedents -= 1;
            let span: SimpleSpan = lex.span().into();
            return Some((Token::Dedent, span));
        }
        if lex.extras.pending_indent {
            lex.extras.pending_indent = false;
            let span: SimpleSpan = lex.span().into();
            return Some((Token::Indent, span));
        }

        loop {
            let lex = self.lexer_mut();
            let next = lex.next()?;
            let span: SimpleSpan = lex.span().into();

            match next {
                Ok(Token::FormatStringDelim) => {
                    // Enter format string sub-lexer
                    self.lex_format_string(span);
                    return self.pending.pop_front();
                }
                Ok(tok) => {
                    if lex.extras.indent_overflow {
                        lex.extras.indent_overflow = false;
                        self.errors.push((LexSymptom::OverflowIndent, span));
                    }
                    return Some((tok, span));
                }
                Err(err) => {
                    self.errors.push((err.clone(), span));
                    let lex = self.lexer_mut();
                    match err {
                        LexSymptom::UnexpectedCharacter => {
                            // Skip one Unicode scalar value.
                            let skip = match lex.remainder().as_bytes().first() {
                                None => break None,
                                Some(&b) if b.is_ascii() => 1,
                                Some(&b) => b.leading_ones() as usize,
                            };
                            lex.bump(skip);
                        }
                        _ => {
                            // InvalidInteger | InvalidFloat - Lexer already consumed the token, nothing to skip
                            // UnclosedString is handled as a token variant, not a lex error
                        }
                    }
                }
            }
        }
    }

    fn lex_format_string(&mut self, open_span: SimpleSpan) {
        let main_lexer = self.inner.take().expect("lexer present");
        let result = FormatStringLexer::lex(main_lexer, open_span);
        self.pending.extend(result.tokens);
        self.errors.extend(result.errors);
        self.inner = Some(result.main_lexer);
    }

    /// `next_raw` includes comments
    pub fn next_raw(&mut self) -> Option<(Token<'src>, SimpleSpan)> {
        if let Some(item) = self.next_with_indent() {
            return Some(item);
        }

        // EOF: flush remaining open indent levels as Dedents.
        let lex = self.lexer_mut();
        let dedent_count = lex.extras.indent_depth as usize - 1;
        if dedent_count > 0 {
            lex.extras.indent_depth = 1;
            lex.extras.pending_dedents = dedent_count as u8;
            self.next_with_indent()
        } else {
            None
        }
    }
}

// TODO: move this to whatever is referencing it
/// Returns true if `byte_pos` falls inside a comment token in `source`.
///
/// More accurate than scanning for `--` in source text, which would false-positive
/// on `--` inside string literals.
pub fn is_comment_at(source: &str, byte_pos: usize) -> bool {
    let mut lexer = GinLexer::new(source);
    while let Some((tok, span)) = lexer.next_raw() {
        if matches!(tok, Token::Comment(_)) && span.start <= byte_pos && byte_pos <= span.end {
            return true;
        }
    }
    false
}

impl<'src> Iterator for GinLexer<'src> {
    type Item = (Token<'src>, SimpleSpan);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let item = self.next_raw()?;
            if !matches!(item.0, Token::Comment(_)) {
                return Some(item);
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Estimate ~4 bytes per token so that `collect()` pre-allocates once.
        let lex = self.lexer();
        let pending = lex.extras.pending_dedents as usize
            + usize::from(lex.extras.pending_indent)
            + self.pending.len();
        let from_source = lex.remainder().len() / 4;
        let estimate = from_source + pending;
        (estimate, Some(estimate + estimate / 2))
    }
}
