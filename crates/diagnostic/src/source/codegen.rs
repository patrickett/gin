use crate::{Category, Symptom, SymptomLike};
use chumsky::span::SimpleSpan;

#[derive(Debug, Clone)]
pub enum CodegenSymptom {
    Internal { message: String },
}

impl SymptomLike for CodegenSymptom {
    fn into_symptom(self, span: SimpleSpan) -> Symptom {
        let (code, message, help) = match self {
            Self::Internal { message: msg } => (
                "codegen-internal",
                msg,
                Some("an internal compiler error occurred".into()),
            ),
        };

        Symptom {
            code,
            message,
            help,
            span,
            category: Category::Flaw,
        }
    }
}
