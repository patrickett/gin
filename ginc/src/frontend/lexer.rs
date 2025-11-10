//! The lexer is handled by [logos](https://github.com/maciejhirsz/logos)

// TODO: replace logos lexer with handwritten
// we can assume lowercase word is id and first letter upper is tag

use logos::{Lexer, Logos};
use std::{collections::VecDeque, ops::Range};

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

// NOTE: sort everything by longest first
// PERF: remove regex and replace with isFirstLetterCapital
#[derive(Logos, Debug, PartialEq, Clone)]
#[logos(extras = Extras)]
pub enum Token<'src> {
    // Control flow
    #[token("continue")]
    Continue,
    #[token("derives")]
    Derives,
    #[token("private")]
    Private,
    #[token("public")]
    Public,
    #[token("define")]
    Define,
    #[token("return")]
    Return,
    #[token("break")]
    Break,
    #[token("alias")]
    Alias,
    #[token("macro")]
    Macro,
    #[token("needs")]
    Needs,
    #[token("then")]
    Then,
    #[token("when")]
    When,
    #[token("does")]
    Does,
    #[token("from")]
    From,
    #[token("for")]
    For,
    #[token("use")]
    Use,
    #[token("has")]
    Has,
    #[token("and")]
    And,
    #[token("def")]
    Def,

    // Keywords first
    #[token("where")]
    Where,
    #[token("as")]
    As,
    #[token("do")]
    Do,
    #[token("if")]
    If,
    #[token("in")]
    In,
    #[token("is")]
    Is,
    #[token("of")]
    Of,
    #[token("or")]
    Or,

    // ids
    #[regex("[A-Z][a-zA-Z]*(?:[A-Z][a-zA-Z]*)*")]
    Tag(&'src str),
    #[regex("(_?[a-z]+(_[a-z]+)*)")]
    Id(&'src str),

    // Match floating-point numbers first
    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().parse::<f64>().unwrap())]
    Float(f64),

    // Then match integers
    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i64>().unwrap())]
    Int(i64),
    // Numbers
    // #[regex("[0-9]+")]
    // Number,

    // Strings
    String(&'src str),
    // Normal comment (skipped)
    // #[regex(r"--[^\n]*", logos::skip)]
    // Comment,
    #[token("...")]
    Ellipsis,
    #[token("::=")]
    IsReplacedBy,
    #[token(":=")]
    Assignment,
    #[token("|-")]
    /// https://en.wikipedia.org/wiki/Turnstile_(symbol)
    Turnstile,
    // Operators (longest first)
    #[token("==")]
    EqEq,
    #[token("--")]
    DashDash,
    #[token("---")]
    DashDashDash,
    #[token("!=")]
    NotEq,
    #[token("<=")]
    LessEq,
    #[token(">=")]
    GreaterEq,
    #[token("=")]
    Equals,
    #[token("<")]
    Less,
    #[token(">")]
    Greater,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("\\")]
    SlashOr,
    #[token("/")]
    Slash,
    #[token("|")]
    Bar,
    #[token("^")]
    Caret,
    #[token("~")]
    Tilde,
    // Punctuation
    #[token(".")]
    Dot,
    #[token("#")]
    Pound,
    #[token(":")]
    Colon,
    #[token(";")]
    ColonSemi,
    #[token("(")]
    ParenOpen,
    #[token(")")]
    ParenClose,
    #[token("[")]
    BracketOpen,
    #[token("]")]
    BracketClose,
    #[token("{")]
    CurlyOpen,
    #[token("}")]
    CurlyClose,
    #[token("&")]
    Ampersand,
    #[token(",")]
    Comma,

    // Newline triggers indentation logic
    #[regex(r"\n", handle_newline)]
    Newline,

    // Indentation tokens
    Indent,
    Dedent,

    // Inline whitespace (skip, but only non-leading)
    #[regex(r"[ \t]+", logos::skip)]
    Whitespace,
    Error,
}

// fn handle_comment<'src>(lex: &mut Lexer<'src, Token<'src>>) -> Token<'src> {
//     // Consume everything up to but not including the newline.
//     let mut consumed = 0usize;
//     for ch in lex.remainder().chars() {
//         if ch == '\n' {
//             break;
//         }
//         consumed += ch.len_utf8();
//     }
//     lex.bump(consumed);
//     Token::Comment
// }

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
    pub fn new(src: &'src str) -> Self {
        Self {
            inner: Token::lexer_with_extras(src, Extras::default()),
        }
    }

    fn next_with_indent(&mut self) -> Option<(Token<'src>, Range<usize>)> {
        // Check if there are pending indent/dedent tokens
        if let Some(tok) = self.inner.extras.pending.pop_front() {
            let span = self.inner.span(); // reuse last span or synthesize
            return Some((
                match tok {
                    IndentToken::Indent => Token::Indent,
                    IndentToken::Dedent => Token::Dedent,
                },
                span,
            ));
        }

        // Otherwise, read next Logos token and track its span
        let start = self.inner.span().end; // where we left off
        let next = self.inner.next()?;
        let end = self.inner.span().end;
        let span = start..end;

        match next {
            Ok(tok) => Some((tok, span)),
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
    type Item = (Token<'src>, Range<usize>);

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

        item
    }
}
