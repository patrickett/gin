//! Parse diagnostic variant.

use crate::diagnostic::{Category, Symptom, SymptomDetail, SymptomSource};
use chumsky::span::SimpleSpan;

pub enum ParseSymptom {
    InvalidSyntax,
    Custom(String),
    EmptyParens { suggested: String },
    UnusedValue { value: String },
}

impl SymptomDetail for ParseSymptom {
    fn id(&self) -> u8 {
        match self {
            ParseSymptom::InvalidSyntax => 1,
            ParseSymptom::Custom(_) => 2,
            ParseSymptom::EmptyParens { .. } => 3,
            ParseSymptom::UnusedValue { .. } => 4,
        }
    }

    fn message(&self) -> String {
        match self {
            ParseSymptom::InvalidSyntax => "invalid syntax".into(),
            ParseSymptom::Custom(msg) => msg.clone(),
            ParseSymptom::EmptyParens { .. } => "empty parentheses are not needed".into(),
            ParseSymptom::UnusedValue { value } => {
                format!("unused value: `{value}`")
            }
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            ParseSymptom::EmptyParens { suggested } => {
                Some(format!("remove the parentheses: `{suggested}`"))
            }
            ParseSymptom::UnusedValue { .. } => {
                Some("did you mean to indent this as part of the previous expression?".into())
            }
            _ => None,
        }
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

pub fn empty_parens_hint(suggested: String, span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Parse(ParseSymptom::EmptyParens { suggested }),
        span,
        category: Category::Help,
    }
}

pub fn unused_value(value: String, span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Parse(ParseSymptom::UnusedValue { value }),
        span,
        category: Category::Info,
    }
}
