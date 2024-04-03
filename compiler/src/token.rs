use crate::expr::literal::Literal;

#[derive(Debug, PartialEq, Clone)]
pub enum Keyword {
    Include,
    When,
    For,
    Return,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Token {
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
