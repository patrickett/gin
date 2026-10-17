use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum LexCode {
    InvalidFloat,
    InvalidInteger,
    OverflowIndent,
    UnclosedString,
    UnexpectedCharacter,
}

impl Prefix for LexCode {
    fn prefix(&self) -> &'static str {
        "lex"
    }
}

impl Code for LexCode {
    fn name(&self) -> &'static str {
        match self {
            Self::InvalidFloat => "invalid-float",
            Self::InvalidInteger => "invalid-integer",
            Self::OverflowIndent => "overflow-indent",
            Self::UnclosedString => "unclosed-string",
            Self::UnexpectedCharacter => "unexpected-character",
        }
    }
}

impl LexCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::InvalidFloat,
            Self::InvalidInteger,
            Self::OverflowIndent,
            Self::UnclosedString,
            Self::UnexpectedCharacter,
        ]
        .into_iter()
    }
}
