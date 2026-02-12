//! Type diagnostic variant.

use crate::diagnostic::{Category, Symptom, SymptomDetail, SymptomSource};
use chumsky::span::SimpleSpan;

pub enum TypeSymptom {
    Mismatch,
    Unknown,
    InferenceFailed,
}

impl SymptomDetail for TypeSymptom {
    fn id(&self) -> u8 {
        match self {
            TypeSymptom::Mismatch => 1,
            TypeSymptom::Unknown => 2,
            TypeSymptom::InferenceFailed => 3,
        }
    }

    fn message(&self) -> String {
        match self {
            TypeSymptom::Mismatch => "type mismatch",
            TypeSymptom::Unknown => "unknown type",
            TypeSymptom::InferenceFailed => "failed to infer type",
        }
        .into()
    }

    fn help(&self) -> Option<String> {
        match self {
            TypeSymptom::Mismatch => Some("types do not match".into()),
            TypeSymptom::Unknown => Some("type is not defined".into()),
            TypeSymptom::InferenceFailed => Some("could not infer the type".into()),
        }
    }
}

pub const fn mismatch(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::Mismatch),
        span,
        category: Category::Flaw,
    }
}

pub const fn unknown(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::Unknown),
        span,
        category: Category::Flaw,
    }
}

pub const fn inference_failed(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::InferenceFailed),
        span,
        category: Category::Flaw,
    }
}
