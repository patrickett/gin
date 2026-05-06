use crate::{CodegenSymptom, IoSymptom, LexSymptom, ParseSymptom, TypeSymptom, UseSymptom};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticCode {
    Import(UseSymptom),
    Lex(LexSymptom),
    Parse(ParseSymptom),
    Type(TypeSymptom),
    Io(IoSymptom),
    Codegen(CodegenSymptom),
}

impl From<UseSymptom> for DiagnosticCode {
    fn from(v: UseSymptom) -> Self {
        DiagnosticCode::Import(v)
    }
}
impl From<LexSymptom> for DiagnosticCode {
    fn from(v: LexSymptom) -> Self {
        DiagnosticCode::Lex(v)
    }
}
impl From<ParseSymptom> for DiagnosticCode {
    fn from(v: ParseSymptom) -> Self {
        DiagnosticCode::Parse(v)
    }
}
impl From<TypeSymptom> for DiagnosticCode {
    fn from(v: TypeSymptom) -> Self {
        DiagnosticCode::Type(v)
    }
}
impl From<IoSymptom> for DiagnosticCode {
    fn from(v: IoSymptom) -> Self {
        DiagnosticCode::Io(v)
    }
}
impl From<CodegenSymptom> for DiagnosticCode {
    fn from(v: CodegenSymptom) -> Self {
        DiagnosticCode::Codegen(v)
    }
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
        }
    }

    /// Delegate custom rendering to the domain type.
    /// Returns `true` if the domain handled printing itself.
    pub fn render_custom(
        &self,
        diag: &crate::Diagnostic,
        span_table: &crate::SpanTable,
        source: &str,
        filename: &str,
    ) -> bool {
        match self {
            DiagnosticCode::Lex(lex) => lex.render_custom(diag, span_table, source, filename),
            _ => false,
        }
    }
}
