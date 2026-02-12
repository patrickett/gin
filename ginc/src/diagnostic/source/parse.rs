//! Parse diagnostic variant.

use crate::diagnostic::{Category, Symptom, SymptomDetail, SymptomSource};
use chumsky::span::SimpleSpan;

pub enum ParseSymptom {
    InvalidSyntax,
    Custom(String),
}

impl SymptomDetail for ParseSymptom {
    fn id(&self) -> u8 {
        match self {
            ParseSymptom::InvalidSyntax => 1,
            ParseSymptom::Custom(_) => 2,
        }
    }

    fn message(&self) -> String {
        match self {
            ParseSymptom::InvalidSyntax => "invalid syntax".into(),
            ParseSymptom::Custom(msg) => msg.clone(),
        }
    }

    fn help(&self) -> Option<String> {
        None
    }
}

pub const fn unexpected_token(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Parse(ParseSymptom::InvalidSyntax),
        span,
        category: Category::Flaw,
    }
}

pub fn custom(msg: String, span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Parse(ParseSymptom::Custom(msg)),
        span,
        category: Category::Flaw,
    }
}
