use strum::AsRefStr;

use crate::SpanId;
use crate::{Category, Symptom, SymptomCode, SymptomLike};

#[derive(Debug, Clone, PartialEq, Eq, AsRefStr)]
pub enum IoSymptom {
    #[strum(to_string = "io-read-failed")]
    ReadFailed,
    #[strum(to_string = "io-write-failed")]
    WriteFailed,
    #[strum(to_string = "io-resolution-failed")]
    ResolutionFailed,
}

impl SymptomLike for IoSymptom {
    fn into_symptom(self, span_id: SpanId) -> Symptom {
        let (message, help): (&str, &str) = match self {
            Self::ReadFailed => (
                "failed to read file",
                "check if the file exists and you have permission to read it",
            ),
            Self::WriteFailed => (
                "failed to write file",
                "check if you have permission to write to this location",
            ),
            Self::ResolutionFailed => (
                "failed to resolve import",
                "check if the import path is correct",
            ),
        };

        Symptom {
            code: SymptomCode::Io(self),
            message: message.into(),
            help: Some(help.into()),
            span_id,
            category: Category::Flaw,
        }
    }
}
