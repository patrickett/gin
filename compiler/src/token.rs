#[derive(Debug, PartialEq, Clone)]
pub enum Literal {
    Bool(bool),
    String(String),
    Number(usize),
}

#[derive(Debug, PartialEq, Clone)]
pub struct Token {
    pos: usize,
    // end: usize,
    kind: TokenKind,
}

impl Token {
    pub fn new(kind: TokenKind, pos: usize) -> Token {
        Self {
            pos,
            // end: start + length,
            kind,
        }
    }

    pub fn kind(&self) -> TokenKind {
        self.kind.to_owned()
    }

    pub fn pos(&self) -> usize {
        self.pos.to_owned()
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Keyword {
    If,
    Else,
    For,
    Return,
}

#[derive(Debug, PartialEq, Clone)]
pub enum TokenKind {
    ParenOpen,
    ParenClose,
    CurlyOpen,
    CurlyClose,
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
    Literal(Literal),
    LessThan,
    GreaterThan,
    RightArrow,
    Plus,
    Dash,
    Equals,
    Ampersand,
    Star,
    Percent,
    Keyword(Keyword),
}
