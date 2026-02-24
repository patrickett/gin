//! Lexer diagnostic variant.

use crate::diagnostic::{Category, Symptom, SymptomDetail, SymptomSource};
use chumsky::span::SimpleSpan;

#[derive(Default, Debug, Clone, PartialEq)]
pub enum LexSymptom {
    UnclosedString,
    InvalidInteger,
    InvalidFloat,
    #[default]
    UnexpectedCharacter,
}

impl From<std::num::ParseIntError> for LexSymptom {
    fn from(_: std::num::ParseIntError) -> Self {
        LexSymptom::InvalidInteger
    }
}

impl From<std::num::ParseFloatError> for LexSymptom {
    fn from(_: std::num::ParseFloatError) -> Self {
        LexSymptom::InvalidFloat
    }
}

impl SymptomDetail for LexSymptom {
    fn id(&self) -> u8 {
        match self {
            LexSymptom::UnclosedString => 1,
            LexSymptom::InvalidInteger => 2,
            LexSymptom::InvalidFloat => 3,
            LexSymptom::UnexpectedCharacter => 4,
        }
    }

    fn message(&self) -> String {
        match self {
            LexSymptom::UnclosedString => "unclosed string literal".into(),
            LexSymptom::InvalidInteger => "integer literal out of range".into(),
            LexSymptom::InvalidFloat => "float literal out of range".into(),
            LexSymptom::UnexpectedCharacter => "unexpected character".into(),
        }
    }

    fn help(&self) -> Option<String> {
        None
    }
}

pub const fn unclosed_string(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Lex(LexSymptom::UnclosedString),
        span,
        category: Category::Flaw,
    }
}

pub const fn invalid_integer(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Lex(LexSymptom::InvalidInteger),
        span,
        category: Category::Flaw,
    }
}

pub const fn invalid_float(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Lex(LexSymptom::InvalidFloat),
        span,
        category: Category::Flaw,
    }
}

pub const fn unexpected_character(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Lex(LexSymptom::UnexpectedCharacter),
        span,
        category: Category::Flaw,
    }
}
