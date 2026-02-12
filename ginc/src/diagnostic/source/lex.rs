//! Lexer diagnostic variant.

use crate::diagnostic::{Category, Symptom, SymptomDetail, SymptomSource};
use chumsky::span::SimpleSpan;

pub enum LexSymptom {
    UnclosedString,
}

impl SymptomDetail for LexSymptom {
    fn id(&self) -> u8 {
        match self {
            LexSymptom::UnclosedString => 1,
        }
    }

    fn message(&self) -> String {
        match self {
            LexSymptom::UnclosedString => "unclosed string literal".into(),
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
