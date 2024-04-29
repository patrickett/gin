use crate::ngin::value::GinValue;
use std::fmt;

#[derive(Debug, PartialEq, Clone)]
pub enum Keyword {
    Is,
    If,
    Else,
    Then,
    Include,
    When,
    For,
    Return,
}

#[derive(PartialEq, Clone)]
pub struct Token {
    /// The line on which the token is on
    line: usize,
    /// The index of the first char of the token
    start: usize,
    end: usize,
    kind: TokenKind,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "[{}:{}-{}] {}",
            self.line, self.start, self.end, self.kind
        )
    }
}

impl Token {
    pub fn kind(&self) -> &TokenKind {
        &self.kind
    }

    pub const fn new(kind: TokenKind, line: usize, start: usize, end: usize) -> Self {
        Self {
            kind,
            line,
            start,
            end,
        }
    }

    pub fn position(&self) -> String {
        format!("{}:{}", self.line, self.start)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum TokenKind {
    /// Range(start, end)
    Range(usize, usize),
    ParenOpen,
    ParenClose,
    CurlyOpen,
    CurlyClose,
    BracketOpen,
    BracketClose,
    SlashBack,
    SlashForward,
    Colon,
    SemiColon,
    Comma,
    Tab,
    Space,
    Newline,
    Comment(String),
    DocComment(String),
    Id(String),
    Tag(String),
    Literal(GinValue),
    LessThan,
    LessThanOrEqualTo,
    GreaterThan,
    GreaterThanOrEqualTo,
    RightArrow,
    LeftArrow,
    Plus,
    Dash,
    Equals,
    Ampersand,
    Star,
    Percent,
    Keyword(Keyword),
}
