//! The lexer is handled by [logos](https://github.com/maciejhirsz/logos)

// TODO: replace logos lexer with handwritten for better performance
// PERF: Handwritten lexer can be optimized for specific Gin syntax patterns
// we can assume lowercase word is id and first letter upper is tag

mod semantic_token_type;
mod token;
mod tokenize;

use chumsky::span::SimpleSpan;
use logos::{Lexer, Logos};
use std::collections::VecDeque;

pub use semantic_token_type::*;
pub use token::*;
pub use tokenize::*;

/// Synthetic tokens for indentation handling
#[derive(Debug, Clone, PartialEq)]
pub enum IndentToken {
    Indent,
    Dedent,
}

/// Extras for the lexer: indentation stack and pending tokens
#[derive(Debug)]
pub struct Extras {
    indent_stack: Vec<usize>,
    pending: VecDeque<IndentToken>,
}

impl Default for Extras {
    fn default() -> Self {
        Self {
            indent_stack: vec![0],
            pending: VecDeque::new(),
        }
    }
}

/// Callback for newlines: measure indentation and enqueue Indent/Dedent
fn handle_newline<'src>(lex: &mut Lexer<'src, Token<'src>>) -> Token<'src> {
    let remainder = lex.remainder();
    let mut count = 0;
    for c in remainder.chars() {
        match c {
            ' ' => count += 1,
            '\t' => count += 4, // tab width
            _ => break,
        }
    }
    lex.bump(count);

    let extras = &mut lex.extras;
    let current = *extras.indent_stack.last().unwrap_or(&0);

    if count > current {
        extras.indent_stack.push(count);
        extras.pending.push_back(IndentToken::Indent);
    } else if count < current {
        while extras.indent_stack.last().copied().unwrap_or(0) > count {
            extras.indent_stack.pop();
            extras.pending.push_back(IndentToken::Dedent);
        }
    }

    Token::Newline
}

pub struct GinLexer<'src> {
    pub inner: Lexer<'src, Token<'src>>,
}

impl<'src> GinLexer<'src> {
    pub fn new_owned(src: String) -> Self {
        // We need to extend the lifetime of src to 'src
        let boxed: Box<str> = src.into_boxed_str();
        let leaked = Box::leak(boxed);
        Self {
            inner: Token::lexer_with_extras(leaked, Extras::default()),
        }
    }

    pub fn new(src: &'src str) -> Self {
        Self {
            inner: Token::lexer_with_extras(src, Extras::default()),
        }
    }

    fn next_with_indent(&mut self) -> Option<(Token<'src>, SimpleSpan)> {
        // Check if there are pending indent/dedent tokens
        if let Some(tok) = self.inner.extras.pending.pop_front() {
            let span = self.inner.span(); // reuse last span or synthesize
            let simple_span: SimpleSpan = span.into();

            return Some((
                match tok {
                    IndentToken::Indent => Token::Indent,
                    IndentToken::Dedent => Token::Dedent,
                },
                simple_span,
            ));
        }

        // Otherwise, read next Logos token and track its span
        let start = self.inner.span().end; // where we left off
        let next = self.inner.next()?;
        let end = self.inner.span().end;
        let span = start..end;

        let simple_span: SimpleSpan = span.into();

        match next {
            Ok(tok) => Some((tok, simple_span)),
            Err(()) => {
                let ch = self.inner.remainder().chars().next();
                eprintln!("Unexpected character: {ch:?}");
                self.inner.bump(ch.map(|c| c.len_utf8()).unwrap_or(1));
                self.next_with_indent()
            }
        }
    }
}

impl<'src> Iterator for GinLexer<'src> {
    type Item = (Token<'src>, SimpleSpan);

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.next_with_indent();

        if item.is_none() {
            // flush pending dedents at EOF
            let dedent_count = self.inner.extras.indent_stack.len().saturating_sub(1);
            if dedent_count > 0 {
                for _ in 0..dedent_count {
                    self.inner.extras.indent_stack.pop();
                    self.inner.extras.pending.push_back(IndentToken::Dedent);
                }
                return self.next_with_indent();
            }
        }

        if let Some(item) = item {
            let simple_span: SimpleSpan = item.1;
            Some((item.0, simple_span))
        } else {
            None
        }
    }
}
