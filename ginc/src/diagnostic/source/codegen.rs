//! Codegen diagnostic variant.

use crate::diagnostic::{Category, Symptom, SymptomDetail, SymptomSource};
use chumsky::span::SimpleSpan;

#[derive(Debug)]
pub enum CodegenSymptom {
    Redefinition,
    SelfOutsideMethod { span: SimpleSpan },
    Internal(String),
}

impl SymptomDetail for CodegenSymptom {
    fn id(&self) -> u8 {
        match self {
            CodegenSymptom::Redefinition => 1,
            CodegenSymptom::SelfOutsideMethod { .. } => 3,
            CodegenSymptom::Internal(_) => 2,
        }
    }

    fn message(&self) -> String {
        match self {
            CodegenSymptom::Redefinition => "symbol redefinition".into(),
            CodegenSymptom::SelfOutsideMethod { .. } => "self used outside method".into(),
            CodegenSymptom::Internal(s) => s.clone(),
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            CodegenSymptom::Redefinition => Some("symbol is defined multiple times".into()),
            CodegenSymptom::SelfOutsideMethod { .. } => Some("self can only be used inside methods".into()),
            CodegenSymptom::Internal(_) => Some("an internal compiler error occurred".into()),
        }
    }
}

pub const fn redefinition(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::CodeGen(CodegenSymptom::Redefinition),
        span,
        category: Category::Flaw,
    }
}
