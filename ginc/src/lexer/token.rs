use crate::diagnostic::lex::LexSymptom;
use logos::{Lexer, Logos};

/// Maximum indentation depth supported by the lexer.
pub const MAX_INDENT_DEPTH: usize = 16;

/// Indent and Dedent are mutually exclusive branches per newline.
/// We can exploit this by using `pending_indent` and `pending_dedents`
#[derive(Debug)]
pub struct LexContext {
    pub indent_stack: [u16; MAX_INDENT_DEPTH],
    pub indent_depth: u8,
    /// Dedent tokens still to be emitted, decremented one per call.
    pub pending_dedents: u8,
    /// Whether a single Indent token is waiting to be emitted.
    pub pending_indent: bool,
    /// Flag set when indentation exceeds MAX_INDENT_DEPTH.
    pub indent_overflow: bool,
}

impl Default for LexContext {
    fn default() -> Self {
        Self {
            indent_stack: [0u16; MAX_INDENT_DEPTH],
            indent_depth: 1,
            pending_dedents: 0,
            pending_indent: false,
            indent_overflow: false,
        }
    }
}

/// Handles newline characters and manages indentation state.
/// This function is called by logos when a newline is encountered.
#[inline]
pub fn handle_newline<'src>(lex: &mut Lexer<'src, Token<'src>>) -> Token<'src> {
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
        } else {
            extras.indent_overflow = true;
        }
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

#[derive(Logos, Debug, PartialEq, Clone, Copy)]
#[logos(extras = LexContext)]
#[logos(error = LexSymptom)]
pub enum Token<'src> {
    #[token("continue")]
    Continue,
    #[token("private")]
    Private,
    #[token("return")]
    Return,
    #[token("break")]
    Break,
    #[token("loop")]
    Loop,
    #[token("then")]
    Then,
    #[token("when")]
    When,
    #[token("else")]
    Else,
    #[token("self")]
    SelfInstance,
    #[token("Self")]
    SelfTag,
    #[token("for")]
    For,
    #[token("use")]
    Use,
    #[token("has")]
    Has,
    #[token("and")]
    And,
    #[token("...")]
    Infer,
    #[token("as")]
    As,
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
    // String-bearing variants: logos uses lex.slice() by default for &'src str
    #[regex(r"[A-Z][a-zA-Z]*")]
    Tag(&'src str),
    #[regex(r"_?[a-z]+(?:_[a-z]+)*")]
    Id(&'src str),
    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().parse::<f64>())]
    Float(f64),
    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i64>())]
    Int(i64),
    #[regex(r"'[^'\n]*'", |lex| { let s = lex.slice(); &s[1..s.len()-1] })]
    String(&'src str),
    #[regex(r"'[^'\n]*", |lex| { let s = lex.slice(); &s[1..] })]
    UnterminatedString(&'src str),
    #[token("\"")]
    FormatStringDelim,
    FormatStringText(&'src str),
    FormatInterpStart,
    FormatInterpEnd,
    UnterminatedFormatString,
    #[regex(r"---[^\n]*")]
    DocComment(&'src str),
    #[regex(r"--[^\n]*")]
    Comment(&'src str),
    // Operators (longest first)
    #[token("==")]
    EqEq,
    #[token("/=")]
    NotEq,
    #[token("<=")]
    LessEq,
    #[token("<-")]
    ArrowLeft,
    #[token("->")]
    ArrowRight,
    #[token(">=")]
    GreaterEq,
    #[token("=")]
    Eq,
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
    #[token("^")]
    Caret,
    #[token("~")]
    Tilde,
    #[token(".")]
    Dot,
    #[token("@")]
    At,
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
    #[regex(r"\n", handle_newline)]
    Newline,

    Indent,
    Dedent,

    // TODO: add an error token for unsupported non-ASCII characters outside strings
    #[regex(r"[ \t]+", logos::skip)]
    Whitespace,
}
