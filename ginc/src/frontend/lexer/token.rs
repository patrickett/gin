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

#[derive(Logos, Debug, PartialEq, Clone, Copy)]
#[logos(extras = LexContext)]
pub enum Token<'src> {
    #[token("continue")]
    Continue,
    #[token("private")]
    Private,
    #[token("return")]
    Return,
    #[token("break")]
    Break,
    #[token("Nothing")]
    Nothing,
    #[token("alias")]
    Alias,
    #[token("macro")]
    Macro,
    #[token("needs")]
    Needs,
    #[token("loop")]
    Loop,
    #[token("then")]
    Then,
    #[token("when")]
    When,
    #[token("else")]
    Else,
    #[token("does")]
    Does,
    #[token("from")]
    From,
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
    #[token("where")]
    Where,
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
    #[regex(r"\p{Lu}[\p{L}]*")]
    Tag(&'src str),
    #[regex(r"_?\p{Ll}+(?:_\p{Ll}+)*")]
    Id(&'src str),
    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().parse::<f64>().unwrap())]
    Float(f64),
    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i64>().unwrap())]
    Int(i64),
    #[regex(r"'[^'\n]*'", |lex| { let s = lex.slice(); &s[1..s.len()-1] })]
    String(&'src str),
    #[regex(r"'[^'\n]*", |lex| { let s = lex.slice(); &s[1..] })]
    UnterminatedString(&'src str),
    // Format strings — double-quoted with (var) interpolation
    #[regex(r#""[^"\n]*""#, |lex| { let s = lex.slice(); &s[1..s.len()-1] })]
    FormatString(&'src str),
    #[regex(r#""[^"\n]*"#, |lex| { let s = lex.slice(); &s[1..] })]
    UnterminatedFormatString(&'src str),
    #[regex(r"---[^\n]*")]
    DocComment(&'src str),
    #[regex(r"--[^\n]*")]
    Comment(&'src str),
    #[token("...")]
    Ellipsis,
    #[token("::=")]
    IsReplacedBy,
    #[token("|-")]
    /// https://en.wikipedia.org/wiki/Turnstile_(symbol)
    Turnstile,
    // Operators (longest first)
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEqual,
    #[token("<=")]
    LessEq,
    #[token("<-")]
    ArrowLeft,
    #[token("->")]
    ArrowRight,
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
    #[token("..")]
    DotDot,
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

    // Newline triggers indentation logic
    #[regex(r"\n", handle_newline)]
    Newline,

    Indent,
    Dedent,

    #[regex(r"[ \t]+", logos::skip)]
    Whitespace,
}
