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

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Token<'src> {
    Ampersand,
    And,
    ArrowLeft,
    ArrowRight,
    Asm,
    As,
    At,
    BracketClose,
    BracketOpen,
    Break,
    Caret,
    Colon,
    ColonEq,
    ColonSemi,
    Comma,
    Comment(&'src str),
    Continue,
    CurlyClose,
    CurlyOpen,
    Dedent,
    DocComment(&'src str),
    Dot,
    Else,
    Eq,
    // Operators
    EqEq,
    Extern,
    Float(f64),
    For,
    FormatInterpEnd,
    FormatInterpStart,
    FormatStringDelim,
    FormatStringText(&'src str),
    Greater,
    GreaterEq,
    Has,
    Id(&'src str),
    If,
    In,
    Indent,
    Infer,
    Int(u128),
    Is,
    Less,
    LessEq,
    Loop,
    Minus,
    Newline,
    NotEq,
    Of,
    Or,
    ParenClose,
    ParenOpen,
    Percent,
    Pipe,
    Plus,
    Pound,
    Private,
    Return,
    SelfInstance,
    SelfTag,
    ShiftLeft,
    ShiftRight,
    Slash,
    SlashOr,
    Star,
    String(&'src str),
    // String-bearing variants
    Tag(&'src str),
    Then,
    Tilde,
    UnterminatedFormatString,
    UnterminatedString(&'src str),
    Use,

    When,
    While,

    Whitespace,
}
