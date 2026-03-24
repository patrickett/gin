//! Lexer diagnostic variant.

use crate::diagnostic::{Category, Symptom, SymptomDetail, SymptomSource};
use chumsky::span::SimpleSpan;

// TODO: if we know that indent means its apart of a scope/item above
// can we optomize lexing/parsing in batches. where we split on lines and know
// what lines corrospond to each item based on indent level

// TODO:
// Support:
// ```gin
// Maybe(x) is Some(x) or None
// v: Maybe.Some(4)
// ```

// TODO: change indexing syntax to ().(3)

#[derive(Default, Debug, Clone, PartialEq)]
pub enum LexSymptom {
    UnclosedString,
    InvalidInteger,
    InvalidFloat,
    OverflowIndent,
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
            LexSymptom::OverflowIndent => 5,
            LexSymptom::UnexpectedCharacter => 4,
        }
    }

    fn message(&self) -> String {
        match self {
            LexSymptom::UnclosedString => "unclosed string literal".into(),
            LexSymptom::InvalidInteger => "integer literal out of range".into(),
            LexSymptom::InvalidFloat => "float literal out of range".into(),
            LexSymptom::OverflowIndent => "indentation overflow".into(),
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
