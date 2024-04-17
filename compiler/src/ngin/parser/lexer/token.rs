use crate::ngin::value::GinValue;

#[derive(Debug, PartialEq, Clone)]
pub enum Keyword {
    If,
    Else,
    Then,
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
