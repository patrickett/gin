use strum::AsRefStr;

use crate::SpanId;
use crate::{Category, Diagnostic, DiagnosticCode, DiagnosticLike};

#[derive(Debug, Clone, PartialEq, Eq, AsRefStr)]
pub enum IoSymptom {
    #[strum(to_string = "io-read-failed")]
    ReadFailed,
    #[strum(to_string = "io-write-failed")]
    WriteFailed,
    #[strum(to_string = "io-resolution-failed")]
    ResolutionFailed,
}

impl DiagnosticLike for IoSymptom {
    fn into_diagnostic(self, span_id: SpanId) -> Diagnostic {
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

        Diagnostic {
            code: DiagnosticCode::Io(self),
            message: message.into(),
            help: Some(help.into()),
            span_id,
            category: Category::Flaw,
        }
    }
}
