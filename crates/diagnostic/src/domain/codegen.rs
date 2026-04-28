use strum::AsRefStr;

use crate::DiagnosticLike;

#[derive(Debug, Clone, PartialEq, Eq, AsRefStr)]
pub enum CodegenSymptom {
    #[strum(to_string = "codegen-internal")]
    Internal { message: String },
}

impl DiagnosticLike for CodegenSymptom {
    fn message(&self) -> String {
        match self {
            Self::Internal { message } => message.clone(),
        }
    }

    fn help(&self) -> Option<String> {
        Some("an internal compiler error occurred".into())
    }
}
