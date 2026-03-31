//! IO diagnostic variant.

use crate::{Category, Symptom, SymptomDetail, SymptomSource};
use chumsky::span::SimpleSpan;

pub enum IoSymptom {
    ReadFailed,
    WriteFailed,
    ResolutionFailed,
}

impl SymptomDetail for IoSymptom {
    fn id(&self) -> u8 {
        match self {
            IoSymptom::ReadFailed => 1,
            IoSymptom::WriteFailed => 2,
            IoSymptom::ResolutionFailed => 3,
        }
    }

    fn message(&self) -> String {
        match self {
            IoSymptom::ReadFailed => "failed to read file",
            IoSymptom::WriteFailed => "failed to write file",
            IoSymptom::ResolutionFailed => "failed to resolve import",
        }
        .into()
    }

    fn help(&self) -> Option<String> {
        match self {
            IoSymptom::ReadFailed => {
                Some("check if the file exists and you have permission to read it".into())
            }
            IoSymptom::WriteFailed => {
                Some("check if you have permission to write to this location".into())
            }
            IoSymptom::ResolutionFailed => Some("check if the import path is correct".into()),
        }
    }
}

pub const fn read_failed(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Io(IoSymptom::ReadFailed),
        span,
        category: Category::Flaw,
    }
}

pub const fn write_failed(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Io(IoSymptom::WriteFailed),
        span,
        category: Category::Flaw,
    }
}

pub const fn resolution_failed(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Io(IoSymptom::ResolutionFailed),
        span,
        category: Category::Flaw,
    }
}
