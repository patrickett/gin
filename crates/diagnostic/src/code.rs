use crate::{
    CodegenSymptom, CompileTimeSymptom, IoSymptom, LexSymptom, ParseSymptom, TypeSymptom,
    UseSymptom,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticCode {
    Import(UseSymptom),
    Lex(LexSymptom),
    Parse(ParseSymptom),
    Type(TypeSymptom),
    Io(IoSymptom),
    Codegen(CodegenSymptom),
    CompileTime(CompileTimeSymptom),
}

macro_rules! impl_from_symptom {
    ($($symptom:ident => $variant:ident),* $(,)?) => {
        $(
            impl From<$symptom> for DiagnosticCode {
                fn from(v: $symptom) -> Self {
                    DiagnosticCode::$variant(v)
                }
            }
        )*
    };
}

impl_from_symptom! {
    UseSymptom => Import,
    LexSymptom => Lex,
    ParseSymptom => Parse,
    TypeSymptom => Type,
    IoSymptom => Io,
    CompileTimeSymptom => CompileTime,
    CodegenSymptom => Codegen,
}

impl DiagnosticCode {
    /// Stable kebab-case slug for this diagnostic (e.g. `type-unknown-binding`), not the
    /// outer `DiagnosticCode` variant name.
    pub fn slug(&self) -> &str {
        match self {
            DiagnosticCode::Import(s) => s.as_ref(),
            DiagnosticCode::Lex(s) => s.as_ref(),
            DiagnosticCode::Parse(s) => s.as_ref(),
            DiagnosticCode::Type(s) => s.as_ref(),
            DiagnosticCode::Io(s) => s.as_ref(),
            DiagnosticCode::Codegen(s) => s.as_ref(),
            DiagnosticCode::CompileTime(s) => s.as_ref(),
        }
    }

    /// Delegate custom rendering to the domain type.
    /// Returns `true` if the domain handled printing itself.
    pub fn render_custom(&self, diag: &crate::Diagnostic, source: &str, filename: &str) -> bool {
        match self {
            DiagnosticCode::Lex(lex) => lex.render_custom(diag, source, filename),
            _ => false,
        }
    }
}
