use strum::AsRefStr;

use crate::SpanId;
use crate::{Category, Diagnostic, DiagnosticCode, DiagnosticLike};

#[derive(Debug, Clone, PartialEq, Eq, AsRefStr)]
pub enum CodegenSymptom {
    #[strum(to_string = "codegen-internal")]
    Internal { message: String },
}

impl DiagnosticLike for CodegenSymptom {
    fn into_diagnostic(self, span_id: SpanId) -> Diagnostic {
        let (message, help) = match &self {
            Self::Internal { message: msg } => (
                msg.clone(),
                Some("an internal compiler error occurred".into()),
            ),
        };

        Diagnostic {
            code: DiagnosticCode::Codegen(self),
            message,
            help,
            span_id,
            category: Category::Flaw,
        }
    }
}
