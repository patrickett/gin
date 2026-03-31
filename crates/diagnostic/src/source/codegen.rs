//! Codegen diagnostic variant.

use crate::{Category, Symptom, SymptomDetail, SymptomSource};
use chumsky::span::SimpleSpan;

#[derive(Debug, Clone)]
pub enum CodegenSymptom {
    Redefinition { span: SimpleSpan },
    SelfOutsideMethod { span: SimpleSpan },
    UnknownTag { name: String, span: SimpleSpan },
    Internal { message: String, span: SimpleSpan },
}

impl CodegenSymptom {
    /// Returns the source span associated with this symptom.
    ///
    /// Every codegen symptom carries a span. For symptoms that originate
    /// from a specific source location the span points there; for internal
    /// errors where no source location is available, a zero-width span
    /// (`0..0`) is returned.
    pub fn span(&self) -> SimpleSpan {
        match self {
            CodegenSymptom::Redefinition { span }
            | CodegenSymptom::SelfOutsideMethod { span }
            | CodegenSymptom::UnknownTag { span, .. }
            | CodegenSymptom::Internal { span, .. } => *span,
        }
    }
}

impl SymptomDetail for CodegenSymptom {
    fn id(&self) -> u8 {
        match self {
            CodegenSymptom::Redefinition { .. } => 1,
            CodegenSymptom::SelfOutsideMethod { .. } => 3,
            CodegenSymptom::UnknownTag { .. } => 4,
            CodegenSymptom::Internal { .. } => 2,
        }
    }

    fn message(&self) -> String {
        match self {
            CodegenSymptom::Redefinition { .. } => "symbol redefinition".into(),
            CodegenSymptom::SelfOutsideMethod { .. } => "self used outside method".into(),
            CodegenSymptom::UnknownTag { name, .. } => {
                format!("Unknown tag '{name}' — not declared in any union")
            }
            CodegenSymptom::Internal { message, .. } => message.clone(),
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            CodegenSymptom::Redefinition { .. } => Some("symbol is defined multiple times".into()),
            CodegenSymptom::SelfOutsideMethod { .. } => {
                Some("self can only be used inside methods".into())
            }
            CodegenSymptom::UnknownTag { .. } => {
                Some("declare the tag in a union definition before using it".into())
            }
            CodegenSymptom::Internal { .. } => Some("an internal compiler error occurred".into()),
        }
    }
}

/// Convenience constructor for a redefinition symptom.
pub fn redefinition(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::CodeGen(CodegenSymptom::Redefinition { span }),
        span,
        category: Category::Flaw,
    }
}
