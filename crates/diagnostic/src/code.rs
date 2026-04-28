use strum::AsRefStr;

use crate::{LexSymptom, ParseSymptom, TypeSymptom, IoSymptom, ImportSymptom, CodegenSymptom};

#[derive(Debug, Clone, PartialEq, Eq, AsRefStr)]
pub enum DiagnosticCode {
    Import(ImportSymptom),
    Lex(LexSymptom),
    Parse(ParseSymptom),
    Type(TypeSymptom),
    Io(IoSymptom),
    Codegen(CodegenSymptom),
}

impl From<ImportSymptom> for DiagnosticCode {
    fn from(v: ImportSymptom) -> Self { DiagnosticCode::Import(v) }
}
impl From<LexSymptom> for DiagnosticCode {
    fn from(v: LexSymptom) -> Self { DiagnosticCode::Lex(v) }
}
impl From<ParseSymptom> for DiagnosticCode {
    fn from(v: ParseSymptom) -> Self { DiagnosticCode::Parse(v) }
}
impl From<TypeSymptom> for DiagnosticCode {
    fn from(v: TypeSymptom) -> Self { DiagnosticCode::Type(v) }
}
impl From<IoSymptom> for DiagnosticCode {
    fn from(v: IoSymptom) -> Self { DiagnosticCode::Io(v) }
}
impl From<CodegenSymptom> for DiagnosticCode {
    fn from(v: CodegenSymptom) -> Self { DiagnosticCode::Codegen(v) }
}

impl DiagnosticCode {
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
