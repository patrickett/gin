use crate::{Category, Symptom, SymptomLike};
use chumsky::span::SimpleSpan;

pub enum IoSymptom {
    ReadFailed,
    WriteFailed,
    ResolutionFailed,
}

impl SymptomLike for IoSymptom {
    fn into_symptom(self, span: SimpleSpan) -> Symptom {
        let category = Category::Flaw;
        let code: &str;
        let help: Option<String>;
        let message: String;

        match self {
            Self::ReadFailed => {
                code = "io-read-failed";
                message = "failed to read file".into();
                help = Some("check if the file exists and you have permission to read it".into());
            }
            Self::WriteFailed => {
                code = "io-write-failed";
                message = "failed to write file".into();
                help = Some("check if you have permission to write to this location".into());
            }
            Self::ResolutionFailed => {
                code = "io-resolution-failed";
                message = "failed to resolve import".into();
                help = Some("check if the import path is correct".into());
            }
        }

        Symptom {
            code,
            message,
            help,
            span,
            category,
        }
    }
}
