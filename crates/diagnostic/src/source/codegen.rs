use strum::AsRefStr;

use crate::SpanId;
use crate::{Category, Symptom, SymptomCode, SymptomLike};

#[derive(Debug, Clone, PartialEq, Eq, AsRefStr)]
pub enum CodegenSymptom {
    #[strum(to_string = "codegen-internal")]
    Internal { message: String },
}

impl SymptomLike for CodegenSymptom {
    fn into_symptom(self, span_id: SpanId) -> Symptom {
        let (message, help) = match &self {
            Self::Internal { message: msg } => (
                msg.clone(),
                Some("an internal compiler error occurred".into()),
            ),
        };

        Symptom {
            code: SymptomCode::Codegen(self),
            message,
            help,
            span_id,
            category: Category::Flaw,
        }
    }
}
