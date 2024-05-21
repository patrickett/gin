use std::fmt;

#[derive(Debug, PartialEq, Clone)]
pub enum Keyword {
    Is,
    If,
    Else,
    Then,
    Include,
    When,
    Where,
    For,
    Return,
}

// TODO: Compiler.files.lookup(1) -> "../../..."
// so that we can embed paths in the token (then ast)
// without having to copy strings around

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

#[derive(Debug)]
pub struct Position {
    line: usize,
    start: usize,
}

impl ToString for Position {
    fn to_string(&self) -> String {
        format!("{}:{}", self.line, self.start)
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

    pub fn position(&self) -> Position {
        Position {
            line: self.line,
            start: self.start,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Literal {
    String(String),
    Float(f64),
    Int(u128),
    Nothing,
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
    Literal(Literal),
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

impl TokenKind {
    pub fn len(&self) -> usize {
        match self {
            TokenKind::Range(_, _) => todo!(),
            TokenKind::Comment(cmt) => cmt.len(),
            TokenKind::DocComment(d) => d.len(),
            TokenKind::Id(id) => id.len(),
            TokenKind::Tag(tag) => tag.len(),
            TokenKind::Literal(lit) => match lit {
                Literal::String(s) => s.len(),
                Literal::Float(float) => float.to_string().len(),
                Literal::Int(integer) => integer.to_string().len(),
                Literal::Nothing => 7,
            },
            TokenKind::LessThanOrEqualTo => 2,
            TokenKind::GreaterThanOrEqualTo => 2,
            TokenKind::RightArrow | TokenKind::LeftArrow => 2,
            TokenKind::Keyword(keyword) => match keyword {
                Keyword::Is => 2,
                Keyword::If => 2,
                Keyword::Else => 4,
                Keyword::Then => 4,
                Keyword::Include => 7,
                Keyword::When => 4,
                Keyword::For => 3,
                Keyword::Return => 6,
                Keyword::Where => 5,
            },
            _ => 1,
        }
    }
}
