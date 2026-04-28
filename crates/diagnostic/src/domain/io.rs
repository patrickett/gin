use strum::AsRefStr;

use crate::DiagnosticLike;

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
    fn message(&self) -> String {
        match self {
            Self::ReadFailed => "failed to read file".into(),
            Self::WriteFailed => "failed to write file".into(),
            Self::ResolutionFailed => "failed to resolve import".into(),
        }
    }

    fn help(&self) -> Option<String> {
        Some(match self {
            Self::ReadFailed => "check if the file exists and you have permission to read it",
            Self::WriteFailed => "check if you have permission to write to this location",
            Self::ResolutionFailed => "check if the import path is correct",
        }.into())
    }
}
